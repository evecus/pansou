//! 搜索服务：TG 频道 + 插件并发搜索，结果合并去重与按网盘类型分组

use crate::config;
use crate::model::{MergedLink, MergedLinks, PluginSearchResult, SearchRequest, SearchResponse, SearchResult};
use crate::plugin::PluginManager;
use crate::service::{filter, tg, tg_reachability};
use crate::util::http::{build_client, GLOBAL_CLIENT};
use crate::util::{cache, HttpClient};
use futures::future::join_all;
use std::sync::Arc;
use std::time::Duration;

/// TG 频道搜索整体硬超时（秒）：短于前端超时，保证混合搜索能及时返回
const TG_SEARCH_TIMEOUT_SECS: u64 = 8;
/// 单个 TG 频道客户端超时（秒）：测速接口实测可达即可用，避免慢频道拖垮整批
const TG_CHANNEL_TIMEOUT_SECS: u64 = 8;

pub struct SearchService {
    pub plugin_manager: Arc<PluginManager>,
    client: HttpClient,
    /// 专用于 TG 频道抓取：带代理（对齐 Go 原版 util.GetHTTPClient）、0 重试 + 8s 超时
    /// 旧版用 build_client(None) 无代理，在需要代理的环境 t.me 必然全部超时。
    tg_client: HttpClient,
}

impl SearchService {
    pub fn new(plugin_manager: Arc<PluginManager>) -> Self {
        Self {
            plugin_manager,
            client: HttpClient::new(build_client(None, Duration::from_secs(30)), 1),
            // 用带全局代理的客户端克隆，超时由外层 tokio::time::timeout 控制，0 重试。
            tg_client: HttpClient::new(GLOBAL_CLIENT.clone(), 0),
        }
    }

    pub async fn search(&self, req: &SearchRequest) -> anyhow::Result<SearchResponse> {
        let cfg = config::get();
        let mut source_type = req.src.clone();
        if source_type.is_empty() { source_type = "all".into(); }

        let channels = if req.channels.is_empty() { config::get().default_channels.clone() } else { req.channels.clone() };

        // TG 与插件并发搜索
        // t.me 在国内常不可达，单次抓取 30s×重试会拖垮整个响应；
        // 给 TG 部分套硬超时，超时即放弃 TG 结果，保证插件/频道结果能及时返回。
        let tg_fut = async {
            if source_type == "all" || source_type == "tg" {
                match tokio::time::timeout(
                    Duration::from_secs(TG_SEARCH_TIMEOUT_SECS),
                    self.search_tg(&req.kw, &channels, req.refresh),
                ).await {
                    Ok(r) => r,
                    Err(_) => {
                        tracing::warn!("TG 频道搜索超时({}s)，跳过 TG 结果", TG_SEARCH_TIMEOUT_SECS);
                        vec![]
                    }
                }
            } else { vec![] }
        };
        let plugin_fut = async {
            if (source_type == "all" || source_type == "plugin") && cfg.async_plugin_enabled {
                self.search_plugins(req, &source_type).await
            } else { vec![] }
        };
        let (tg_results, plugin_results) = tokio::join!(tg_fut, plugin_fut);

        // 合并去重
        let all = merge_results(tg_results, plugin_results);

        // 排序
        let mut sorted = all;
        sorted.sort_by(|a, b| b.datetime.cmp(&a.datetime));

        // 构建 merged_by_type（keyword 过滤由插件层与 merge 层完成）
        let cloud_types = req.cloud_types.as_deref();
        let merged = merge_by_type(&sorted, &req.kw, cloud_types);

        // 保留有结果时间的条目
        let filtered_for_results: Vec<SearchResult> = sorted.clone();

        let mut total = match req.res.as_str() {
            "merged_by_type" | "" => merged.values().map(|v| v.len()).sum(),
            _ => filtered_for_results.len(),
        };

        let mut response = SearchResponse {
            total,
            results: Some(filtered_for_results),
            merged_by_type: Some(merged),
        };

        // 按结果类型过滤
        response = filter_by_type(response, &req.res);
        // 应用 include/exclude 过滤
        response = filter::apply_filter(response, &req.filter, &req.res);
        response.total = match req.res.as_str() {
            "results" => response.results.as_ref().map(|v| v.len()).unwrap_or(0),
            "merged_by_type" | "" => response.merged_by_type.as_ref().map(|m| m.values().map(|v| v.len()).sum()).unwrap_or(0),
            _ => response.total,
        };
        total = response.total;
        let _ = total;
        Ok(response)
    }

    async fn search_tg(&self, keyword: &str, channels: &[String], force: bool) -> Vec<SearchResult> {
        let cache_key = format!("tg:{}:{}", keyword, channels.join(","));
        if !force && config::get().cache_enabled {
            if let Some(raw) = cache::main_cache_get(&cache_key) {
                if let Ok(r) = serde_json::from_slice::<Vec<SearchResult>>(&raw) { return r; }
            }
        }

        // TG 可达性门（对齐 Go 原版）：t.me 被墙时不是连接拒绝而是静默丢包，
        // 108 个频道会全部挂满超时、换来 0 条结果，还拖垮 all 模式。
        // 后台探测判定不可达时直接跳过 TG：不写缓存、不计频道失败，让插件结果正常返回。
        if !tg_reachability::reachable() {
            tracing::warn!("[searchTG] {}：t.me 当前不可达，跳过 TG 阶段（{:?}）",
                keyword, tg_reachability::snapshot().get("reason"));
            return vec![];
        }

        let client = &self.tg_client;
        let futs: Vec<_> = channels.iter().map(|ch| {
            let url = tg::build_search_url(ch, keyword, "");
            let ch = ch.clone();
            async move {
                // 每个频道独立超时：慢频道只影响自己，不拖垮整批 join_all
                match tokio::time::timeout(
                    Duration::from_secs(TG_CHANNEL_TIMEOUT_SECS),
                    client.get_text(&url, None),
                ).await {
                    Ok(Ok(html)) => tg::parse_search_results(&html, &ch),
                    Ok(Err(e)) => { tracing::debug!("TG {} 抓取失败: {}", ch, e); vec![] }
                    Err(_) => { tracing::warn!("TG {} 抓取超时({}s)，跳过", ch, TG_CHANNEL_TIMEOUT_SECS); vec![] }
                }
            }
        }).collect();
        let results: Vec<Vec<SearchResult>> = join_all(futs).await;
        let merged: Vec<SearchResult> = results.into_iter().flatten().collect();
        // 空结果不写缓存，避免慢频道/失败时把空结果缓存下来污染后续请求
        if config::get().cache_enabled && !merged.is_empty() {
            cache::main_cache_set(cache_key, serde_json::to_vec(&merged).unwrap_or_default());
        }
        merged
    }

    async fn search_plugins(&self, req: &SearchRequest, source_type: &str) -> Vec<SearchResult> {
        let mut ext = req.ext.clone();
        if req.refresh { ext.insert("refresh".into(), serde_json::Value::Bool(true)); }

        // 选择引擎（请求未显式指定 plugins 时使用全部插件）
        let engines: Vec<_> = if let Some(names) = &req.plugins {
            if names.is_empty() { vec![] } else { self.plugin_manager.filter_by_names(names) }
        } else {
            self.plugin_manager.engines()
        };

        let main_cache_key = format!("plugins:{}:{}", req.kw, req.plugins.as_ref().map(|p| p.join(",")).unwrap_or_default());

        let futs: Vec<_> = engines.into_iter().map(|engine| {
            let ext = ext.clone();
            let kw = req.kw.clone();
            let mck = main_cache_key.clone();
            async move {
                let r: PluginSearchResult = engine.search(&kw, &ext, &mck).await;
                r.results
            }
        }).collect();

        join_all(futs).await.into_iter().flatten().collect()
    }
}

/// 合并 TG 与插件结果（按 unique_id + URL 去重）
fn merge_results(tg: Vec<SearchResult>, plugins: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::with_capacity(tg.len() + plugins.len());
    let mut seen_id = std::collections::HashSet::new();
    let mut seen_url = std::collections::HashSet::new();
    for r in tg.into_iter().chain(plugins.into_iter()) {
        let id_key = if r.unique_id.is_empty() { format!("{}:{}", r.title, r.content) } else { r.unique_id.clone() };
        if !seen_id.insert(id_key) { continue; }
        // 链接级 URL 去重
        let mut links = Vec::new();
        for l in &r.links {
            if seen_url.insert(l.url.clone()) { links.push(l.clone()); }
        }
        let mut r = r;
        r.links = links;
        out.push(r);
    }
    out
}

/// 按网盘类型分组（对应 Go mergeResultsByType）
fn merge_by_type(results: &[SearchResult], keyword: &str, cloud_types: Option<&[String]>) -> MergedLinks {
    let mut merged: MergedLinks = MergedLinks::new();
    let mut unique: std::collections::HashMap<String, MergedLink> = std::collections::HashMap::new();
    let lower_kw = keyword.to_lowercase();

    for r in results {
        for link in &r.links {
            let mut title = if !link.work_title.clone().unwrap_or_default().is_empty() {
                link.work_title.clone().unwrap()
            } else {
                r.title.clone()
            };
            // 关键词过滤（插件可通过 unique_id 前缀声明跳过，此处简化：若标题含关键词则保留）
            if !lower_kw.is_empty() && !title.to_lowercase().contains(&lower_kw) {
                // 对于 skip 插件（unique_id 含 "-" 前缀且在注册表声明）放宽；这里用内容兜底
                if !r.content.to_lowercase().contains(&lower_kw) { continue; }
            }
            if let Some(pos) = title.find("简介").or_else(|| title.find("描述")) {
                title = title[..pos].trim().to_string();
            }
            let source = if !r.channel.is_empty() {
                format!("tg:{}", r.channel)
            } else if let Some(idx) = r.unique_id.find('-') {
                format!("plugin:{}", &r.unique_id[..idx])
            } else { "unknown".into() };

            let dt = link.datetime.unwrap_or(r.datetime);
            let ml = MergedLink {
                url: link.url.clone(), password: link.password.clone(), note: title,
                datetime: dt, source: Some(source), images: r.images.clone(),
            };
            unique.entry(link.url.clone())
                .and_modify(|e| { if ml.datetime > e.datetime { *e = ml.clone(); } })
                .or_insert(ml);
        }
    }

    // 保持原始顺序
    let mut ordered: Vec<(String, String, MergedLink)> = Vec::new();
    let mut added = std::collections::HashSet::new();
    for r in results {
        for link in &r.links {
            if added.insert(link.url.clone()) {
                if let Some(ml) = unique.get(&link.url) {
                    ordered.push((link.r#type.clone(), link.url.clone(), ml.clone()));
                }
            }
        }
    }
    for (t, _, ml) in ordered {
        let t = if t.is_empty() { "unknown".to_string() } else { t };
        merged.entry(t).or_default().push(ml);
    }

    if let Some(types) = cloud_types {
        if !types.is_empty() {
            let allowed: std::collections::HashSet<String> = types.iter().map(|s| s.trim().to_lowercase()).collect();
            merged.retain(|k, _| allowed.contains(&k.to_lowercase()));
        }
    }
    merged
}

fn filter_by_type(resp: SearchResponse, result_type: &str) -> SearchResponse {
    match result_type {
        "merged_by_type" => SearchResponse { total: resp.total, results: None, merged_by_type: resp.merged_by_type },
        "results" => SearchResponse { total: resp.total, results: resp.results, merged_by_type: None },
        "all" => resp,
        _ => SearchResponse { total: resp.total, results: None, merged_by_type: resp.merged_by_type },
    }
}
