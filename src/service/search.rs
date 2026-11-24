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

/// 单个 TG 频道请求超时（秒），对齐 Go 的 TG_CHANNEL_REQUEST_TIMEOUT_SECONDS，默认 4。
fn tg_request_timeout() -> Duration {
    let v = std::env::var("TG_CHANNEL_REQUEST_TIMEOUT_SECONDS").ok()
        .and_then(|v| v.parse::<u64>().ok()).filter(|v| *v > 0).unwrap_or(4);
    Duration::from_secs(v)
}

/// TG 批任务软截止（秒），对齐 Go 的 TG_CHANNEL_TIMEOUT_SECONDS：0/未设置 = 跟随单频道请求超时。
/// 到点就带着"已完成的频道"返回，未完成的频道不再拖住整个响应。
fn tg_batch_timeout() -> Duration {
    let v = std::env::var("TG_CHANNEL_TIMEOUT_SECONDS").ok()
        .and_then(|v| v.parse::<u64>().ok()).filter(|v| *v > 0);
    match v { Some(s) => Duration::from_secs(s), None => tg_request_timeout() }
}

/// TG 部分结果缓存：存"有频道超时/未完成"的结果，短 TTL。
/// 前端首搜后会在 2~3 秒间隔内连续补搜 3 次；没有这层缓存，每次补搜都会重新打 100+ 个频道。
static TG_PARTIAL_CACHE: once_cell::sync::Lazy<moka::sync::Cache<String, Vec<u8>>> =
    once_cell::sync::Lazy::new(|| {
        moka::sync::Cache::builder()
            .time_to_live(Duration::from_secs(120))
            .max_capacity(2_000)
            .build()
    });

pub struct SearchService {
    pub plugin_manager: Arc<PluginManager>,
    client: HttpClient,
    /// 专用于 TG 频道抓取：带代理（对齐 Go 原版 util.GetHTTPClient）、0 重试，超时由每个频道任务自行控制
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

        // TG 与插件并发搜索。
        // TG 阶段自带软截止（默认 4s，对齐 Go）：到点带着已完成频道的结果返回，
        // 因此不再需要外层硬超时——旧版外层 8s 硬超时会在慢频道拖到 8s 时把 *所有* TG 结果丢掉，
        // 且整个响应被拖到 8s，超过前端 5s 的等待窗口，前端就直接显示"未找到相关资源"。
        let tg_fut = async {
            if source_type == "all" || source_type == "tg" {
                self.search_tg(&req.kw, &channels, req.refresh).await
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
        if channels.is_empty() { return vec![]; }
        let cache_key = format!("tg:{}:{}", keyword, channels.join(","));
        let cache_on = config::get().cache_enabled;
        if !force && cache_on {
            if let Some(raw) = cache::main_cache_get(&cache_key) {
                if let Ok(r) = serde_json::from_slice::<Vec<SearchResult>>(&raw) { return r; }
            }
            if let Some(raw) = TG_PARTIAL_CACHE.get(&cache_key) {
                if let Ok(r) = serde_json::from_slice::<Vec<SearchResult>>(&raw) { return r; }
            }
        }

        // TG 可达性门（对齐 Go 原版）：t.me 被墙时直接跳过，不写缓存、不计频道失败。
        if !tg_reachability::reachable() {
            tracing::warn!("[searchTG] {}：t.me 当前不可达，跳过 TG 阶段（{:?}）",
                keyword, tg_reachability::snapshot().get("reason"));
            return vec![];
        }

        let total = channels.len();
        let req_timeout = tg_request_timeout();
        let batch_timeout = tg_batch_timeout();

        // 每个频道一个独立任务，结果经 mpsc 回收；任务自带单频道超时。
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Option<Vec<SearchResult>>>();
        for ch in channels {
            let tx = tx.clone();
            let client = self.tg_client.clone();
            let ch = ch.clone();
            let url = tg::build_search_url(&ch, keyword, "");
            tokio::spawn(async move {
                let r = match tokio::time::timeout(req_timeout, client.get_text(&url, None)).await {
                    Ok(Ok(html)) => Some(tg::parse_search_results(&html, &ch)),
                    Ok(Err(e)) => { tracing::debug!("TG {} 抓取失败: {}", ch, e); None }
                    Err(_) => { tracing::debug!("TG {} 抓取超时({:?})", ch, req_timeout); None }
                };
                let _ = tx.send(r);
            });
        }
        drop(tx);

        // 收集到软截止为止：到点就返回已完成的部分，不等慢频道。
        let mut collected: Vec<SearchResult> = Vec::new();
        let mut done = 0usize;
        let mut succeeded = 0usize;
        let deadline = tokio::time::sleep(batch_timeout);
        tokio::pin!(deadline);
        let mut finished_all = true;
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(r) => {
                        done += 1;
                        if let Some(v) = r { succeeded += 1; collected.extend(v); }
                    }
                    None => break,
                },
                _ = &mut deadline => { finished_all = false; break; }
            }
        }

        tracing::info!("[searchTG] {}：成功 {}/{}，已返回 {}，未完成 {}，共 {} 条",
            keyword, succeeded, total, done, total - done, collected.len());

        if finished_all {
            // 全部频道已回：全失败不写缓存（避免网络抖动的空结果污染缓存）。
            if cache_on && succeeded > 0 && !collected.is_empty() {
                cache::main_cache_set(cache_key, serde_json::to_vec(&collected).unwrap_or_default());
            }
            return collected;
        }

        // 软截止触发：先把已有结果写入短 TTL 缓存，让紧随其后的前端补搜直接命中；
        // 同时后台继续收尾，全部回来后用完整结果覆盖主缓存（对齐 Go 的 backfill）。
        if cache_on && !collected.is_empty() {
            TG_PARTIAL_CACHE.insert(cache_key.clone(), serde_json::to_vec(&collected).unwrap_or_default());
        }
        if cache_on {
            let snapshot = collected.clone();
            tokio::spawn(async move {
                let mut all = snapshot;
                let mut ok_any = succeeded > 0;
                while let Some(r) = rx.recv().await {
                    if let Some(v) = r { ok_any = true; all.extend(v); }
                }
                if ok_any && !all.is_empty() {
                    let mut seen = std::collections::HashSet::new();
                    all.retain(|r| seen.insert(r.unique_id.clone()));
                    cache::main_cache_set(cache_key, serde_json::to_vec(&all).unwrap_or_default());
                }
            });
        }
        collected
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
