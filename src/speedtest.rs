//! 延迟测试模块：仅测 TG 频道 / 插件的响应延迟，供前端搜索配置页展示。
//! 不持久化、不筛选、不参与搜索；每次请求即时测量并返回。

use crate::plugin::{ExtMap, SearchPlugin};
use crate::service::tg;
use crate::util::http::build_client;
use crate::util::HttpClient;
use futures::future::join_all;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 单个目标的延迟结果（latency_ms 为 -1 表示失败/超时）
#[derive(Debug, Clone, Serialize)]
pub struct LatencyResult {
    pub name: String,
    pub latency_ms: i64,
    pub ok: bool,
}

const TEST_KEYWORD: &str = "奔跑吧";

/// 单项测速超时（秒）
pub const TIMEOUT_SECS: u64 = 8;

/// 测单个频道延迟：一次带关键词的真实搜索请求，计时
async fn test_channel(client: &HttpClient, channel: &str, timeout: Duration) -> LatencyResult {
    let url = tg::build_search_url(channel, TEST_KEYWORD, "");
    let start = Instant::now();
    let outcome = tokio::time::timeout(timeout, client.get_text(&url, None)).await;
    match outcome {
        Ok(Ok(_)) => LatencyResult {
            name: channel.to_string(),
            latency_ms: start.elapsed().as_millis() as i64,
            ok: true,
        },
        _ => LatencyResult {
            name: channel.to_string(),
            latency_ms: -1,
            ok: false,
        },
    }
}

/// 测单个插件延迟：一次 search 调用，计时
async fn test_plugin(plugin: &dyn SearchPlugin, client: &HttpClient, timeout: Duration) -> LatencyResult {
    let ext = ExtMap::new();
    let start = Instant::now();
    let outcome = tokio::time::timeout(timeout, plugin.search(client, TEST_KEYWORD, &ext)).await;
    match outcome {
        Ok(Ok(_)) => LatencyResult {
            name: plugin.name().to_string(),
            latency_ms: start.elapsed().as_millis() as i64,
            ok: true,
        },
        _ => LatencyResult {
            name: plugin.name().to_string(),
            latency_ms: -1,
            ok: false,
        },
    }
}

/// 并发测所有频道
pub async fn test_channels(channels: &[String], timeout: Duration) -> Vec<LatencyResult> {
    let client = HttpClient::new(build_client(None, Duration::from_secs(60)), 0);
    let futs: Vec<_> = channels.iter()
        .map(|ch| test_channel(&client, ch, timeout))
        .collect();
    join_all(futs).await
}

/// 并发测所有插件
pub async fn test_plugins(plugins: &[Arc<dyn SearchPlugin>], timeout: Duration) -> Vec<LatencyResult> {
    let client = HttpClient::new(build_client(None, Duration::from_secs(60)), 0);
    let futs: Vec<_> = plugins.iter()
        .map(|p| test_plugin(p.as_ref(), &client, timeout))
        .collect();
    join_all(futs).await
}
