//! 测速模块：对 TG 频道和插件做两步测速（连通性 -> 实际延迟），
//! 结果持久化到 JSON，下次启动只补测新增目标，并按延迟筛选 TOP_N。

use crate::service::tg;
use crate::util::HttpClient;
use crate::util::http::build_client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const TEST_KEYWORD: &str = "奔跑吧";
const REPORT_VERSION: i32 = 1;

/// 测速目标类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Channel,
    Plugin,
}

/// 单个测速结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Result {
    pub name: String,
    pub r#type: SourceType,
    pub reachable: bool,
    pub latency_ms: i64,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tested_at: chrono::DateTime<chrono::Utc>,
}

/// 测速报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub version: i32,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub channels: HashMap<String, Result>,
    pub plugins: HashMap<String, Result>,
}

impl Default for Report {
    fn default() -> Self {
        Self {
            version: REPORT_VERSION,
            updated_at: chrono::Utc::now(),
            channels: HashMap::new(),
            plugins: HashMap::new(),
        }
    }
}

/// 加载已有报告（文件不存在/损坏返回空报告）
pub fn load_report(path: &Path) -> Report {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str::<Report>(&s).unwrap_or_else(|e| {
            println!("[测速] 结果文件解析失败，将重新测速: {}", e);
            Report::default()
        }),
        Err(_) => Report::default(),
    }
}

/// 保存报告（原子写入：先写 tmp 再 rename）
pub fn save_report(path: &Path, report: &Report) -> std::io::Result<()> {
    let mut r = report.clone();
    r.updated_at = chrono::Utc::now();
    let data = serde_json::to_string_pretty(&r).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 测试频道连通性（只判断能否连上、拿到响应）
async fn test_channel_connectivity(client: &HttpClient, channel: &str, timeout: Duration) -> anyhow::Result<()> {
    let url = tg::build_search_url(channel, "", "");
    tokio::time::timeout(timeout, client.get_text(&url, None)).await
        .map_err(|_| anyhow::anyhow!("连通性超时"))??;
    Ok(())
}

/// 测试频道实际延迟（真实搜索请求 + 关键词）
async fn test_channel_latency(client: &HttpClient, channel: &str, timeout: Duration) -> anyhow::Result<Duration> {
    let url = tg::build_search_url(channel, TEST_KEYWORD, "");
    let start = Instant::now();
    tokio::time::timeout(timeout, client.get_text(&url, None)).await
        .map_err(|_| anyhow::anyhow!("延迟测试超时"))??;
    Ok(start.elapsed())
}

/// 对单个频道执行完整两步测速
pub async fn test_channel(client: &HttpClient, channel: &str, timeout: Duration) -> Result {
    let mut result = Result {
        name: channel.to_string(),
        r#type: SourceType::Channel,
        reachable: false,
        latency_ms: 0,
        success: false,
        error: None,
        tested_at: chrono::Utc::now(),
    };
    match test_channel_connectivity(client, channel, timeout).await {
        Ok(_) => result.reachable = true,
        Err(e) => {
            result.error = Some(format!("连通性测试失败: {}", e));
            return result;
        }
    }
    match test_channel_latency(client, channel, timeout).await {
        Ok(d) => {
            result.success = true;
            result.latency_ms = d.as_millis() as i64;
        }
        Err(e) => result.error = Some(format!("延迟测试失败: {}", e)),
    }
    result
}

/// 对单个插件执行完整两步测速（两次独立 search 调用）
pub async fn test_plugin<P>(plugin: &P, timeout: Duration) -> Result
where
    P: crate::plugin::SearchPlugin + ?Sized,
{
    let mut result = Result {
        name: plugin.name().to_string(),
        r#type: SourceType::Plugin,
        reachable: false,
        latency_ms: 0,
        success: false,
        error: None,
        tested_at: chrono::Utc::now(),
    };
    let ext = crate::plugin::ExtMap::new();
    let client = HttpClient::new(build_client(None, timeout), 1);

    // 第一步：连通性
    match tokio::time::timeout(timeout, plugin.search(&client, TEST_KEYWORD, &ext)).await {
        Ok(Ok(_)) => result.reachable = true,
        Ok(Err(e)) => {
            result.error = Some(format!("连通性测试失败: {}", e));
            return result;
        }
        Err(_) => {
            result.error = Some(format!("连通性测试超时({:?})", timeout));
            return result;
        }
    }
    // 第二步：延迟
    let start = Instant::now();
    match tokio::time::timeout(timeout, plugin.search(&client, TEST_KEYWORD, &ext)).await {
        Ok(Ok(_)) => {
            result.success = true;
            result.latency_ms = start.elapsed().as_millis() as i64;
        }
        Ok(Err(e)) => result.error = Some(format!("延迟测试失败: {}", e)),
        Err(_) => result.error = Some(format!("延迟测试超时({:?})", timeout)),
    }
    result
}

/// 运行频道测速并与已有结果合并持久化：只测未测过的目标（串行）。
pub async fn run_and_persist_channels(
    result_path: &Path,
    channels: &[String],
    timeout: Duration,
) -> Report {
    let mut report = load_report(result_path);
    let client = HttpClient::new(build_client(None, Duration::from_secs(60)), 1);
    let mut has_new = false;

    for channel in channels {
        if report.channels.contains_key(channel) {
            continue;
        }
        println!("[测速] 频道 {} 测速中...", channel);
        let r = test_channel(&client, channel, timeout).await;
        if r.success {
            println!("[测速] 频道 {} 完成，延迟: {}ms", channel, r.latency_ms);
        } else {
            println!("[测速] 频道 {} 失败: {:?}", channel, r.error);
        }
        report.channels.insert(channel.clone(), r);
        has_new = true;
    }

    if has_new {
        if let Err(e) = save_report(result_path, &report) {
            println!("[测速] 保存结果文件失败: {}", e);
        } else {
            println!("[测速] 结果已保存至: {}", result_path.display());
        }
    }
    report
}

/// 用插件测速结果列表合并进报告
pub fn merge_plugin_results(result_path: &Path, results: Vec<Result>) -> Report {
    let mut report = load_report(result_path);
    let mut has_new = false;
    for r in results {
        if r.success {
            println!("[测速] 插件 {} 完成，延迟: {}ms", r.name, r.latency_ms);
        } else {
            println!("[测速] 插件 {} 失败: {:?}", r.name, r.error);
        }
        if !report.plugins.contains_key(&r.name) {
            has_new = true;
        }
        report.plugins.insert(r.name.clone(), r);
    }
    if has_new {
        let _ = save_report(result_path, &report);
    }
    report
}

/// 通用 TOP_N 筛选：成功结果按延迟升序优先，不足用失败/未测的补齐
pub fn select_top_n(all_names: &[String], results: &HashMap<String, Result>, top_n: usize) -> Vec<String> {
    if top_n == 0 || top_n >= all_names.len() {
        return all_names.to_vec();
    }
    let mut success_names = Vec::new();
    let mut other_names = Vec::new();
    for name in all_names {
        if let Some(r) = results.get(name) {
            if r.success { success_names.push(name.clone()); } else { other_names.push(name.clone()); }
        } else {
            other_names.push(name.clone());
        }
    }
    success_names.sort_by_key(|n| results.get(n).map(|r| r.latency_ms).unwrap_or(i64::MAX));
    let mut selected = success_names;
    if selected.len() < top_n {
        let need = (top_n - selected.len()).min(other_names.len());
        selected.extend_from_slice(&other_names[..need]);
    }
    selected.truncate(top_n);
    selected
}

pub fn select_top_channels(report: &Report, candidates: &[String], top_n: usize) -> Vec<String> {
    select_top_n(candidates, &report.channels, top_n)
}

pub fn select_top_plugin_names(report: &Report, candidates: &[String], top_n: usize) -> Vec<String> {
    select_top_n(candidates, &report.plugins, top_n)
}
