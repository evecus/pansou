//! 异步插件引擎：实现 Go BaseAsyncPlugin 的缓存、工作池限流、响应超时、后台合并逻辑

use crate::config;
use crate::model::{PluginSearchResult, SearchResult};
use crate::util::{cache, HttpClient};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

pub type ExtMap = serde_json::Map<String, serde_json::Value>;

/// 插件统一接口
#[async_trait]
pub trait SearchPlugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn priority(&self) -> i32 { 3 }
    fn skip_service_filter(&self) -> bool { false }
    /// 底层搜索实现（对应 Go 的 searchFunc）
    async fn search(&self, client: &HttpClient, keyword: &str, ext: &ExtMap) -> Result<Vec<SearchResult>>;
}

/// 缓存条目（进程内，对应 Go 的 apiResponseCache）
#[derive(Clone, Serialize, Deserialize)]
struct CachedResponse {
    results: Vec<SearchResult>,
    timestamp: i64,
    complete: bool,
    access_count: u32,
}

/// 插件响应缓存：按配置存内存或磁盘（见 util::cache::KvCache）
struct ApiCache(cache::KvCache);
impl ApiCache {
    fn get(&self, k: &str) -> Option<CachedResponse> { self.0.get_json(k) }
    fn insert(&self, k: String, v: CachedResponse) { self.0.set_json(&k, &v); }
}
static API_CACHE: once_cell::sync::Lazy<ApiCache> = once_cell::sync::Lazy::new(|| {
    ApiCache(cache::KvCache::new("plugin", Duration::from_secs(config::get().async_cache_ttl_hours * 3600), 20_000))
});

/// 工作池：限流后台任务
struct WorkerPool {
    semaphore: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    max_tasks: usize,
}
impl WorkerPool {
    fn new() -> Self {
        let cfg = config::get();
        Self {
            semaphore: Arc::new(Semaphore::new(cfg.async_max_background_workers)),
            active: Arc::new(AtomicUsize::new(0)),
            max_tasks: cfg.async_max_background_tasks,
        }
    }
    fn try_acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        if self.active.load(Ordering::Relaxed) >= self.max_tasks { return None; }
        match self.semaphore.clone().try_acquire_owned() {
            Ok(p) => { self.active.fetch_add(1, Ordering::Relaxed); Some(p) }
            Err(_) => None,
        }
    }
    fn release(&self) { self.active.fetch_sub(1, Ordering::Relaxed); }
}
static POOL: once_cell::sync::Lazy<WorkerPool> = once_cell::sync::Lazy::new(WorkerPool::new);

/// 异步引擎（对应 Go 的 BaseAsyncPlugin.AsyncSearchWithResult）
pub struct AsyncEngine {
    plugin: Arc<dyn SearchPlugin>,
    /// 短超时客户端（快速响应）
    fast_client: HttpClient,
    /// 长超时客户端（后台处理）
    background_client: HttpClient,
    cache_ttl: Duration,
    response_timeout: Duration,
    /// 最终缓存更新去重追踪
    final_tracker: Arc<Mutex<HashMap<String, bool>>>,
}

impl AsyncEngine {
    pub fn new(plugin: Arc<dyn SearchPlugin>) -> Self {
        let cfg = config::get();
        Self {
            plugin,
            fast_client: HttpClient::new(crate::util::http::build_client(None, cfg.async_response_timeout), 1),
            background_client: HttpClient::new(crate::util::http::build_client(None, cfg.plugin_timeout), 2),
            cache_ttl: Duration::from_secs(cfg.async_cache_ttl_hours * 3600),
            response_timeout: cfg.async_response_timeout,
            final_tracker: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn cache_key(&self, keyword: &str) -> String {
        format!("{}:{}", self.plugin.name(), keyword)
    }

    /// 执行搜索（带缓存与超时）
    pub async fn search(&self, keyword: &str, ext: &ExtMap, main_cache_key: &str) -> PluginSearchResult {
        let ck = self.cache_key(keyword);
        let force = ext.get("refresh").and_then(|v| v.as_bool()).unwrap_or(false);

        // 1. 检查缓存
        if !force {
            if let Some(cached) = API_CACHE.get(&ck) {
                let age = Utc::now().timestamp() - cached.timestamp;
                if age < self.cache_ttl.as_secs() as i64 && cached.complete {
                    // 接近过期时后台刷新
                    if age > (self.cache_ttl.as_secs() as i64 * 4 / 5) {
                        self.spawn_refresh(keyword.to_string(), ck.clone(), ext.clone(), main_cache_key.to_string(), cached.clone());
                    }
                    return PluginSearchResult {
                        results: cached.results, is_final: true, timestamp: Utc::now(),
                        source: self.plugin.name().to_string(), message: "从缓存获取".into(),
                    };
                }
                if !cached.results.is_empty() {
                    self.spawn_refresh(keyword.to_string(), ck.clone(), ext.clone(), main_cache_key.to_string(), cached.clone());
                    return PluginSearchResult {
                        results: cached.results, is_final: false, timestamp: Utc::now(),
                        source: self.plugin.name().to_string(), message: "缓存已过期，后台刷新中".into(),
                    };
                }
            }
        }

        // 2. 启动后台搜索任务
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<Vec<SearchResult>>>();
        let plugin = self.plugin.clone();
        let kw = keyword.to_string();
        let ext2 = ext.clone();
        let pool = &*POOL;
        let permit = pool.try_acquire();
        let use_fast = permit.is_none();
        let client = if use_fast { self.fast_client.clone() } else { self.background_client.clone() };
        let deadline = Instant::now();

        tokio::spawn(async move {
            let _permit = permit; // 持有 permit 直到完成
            let result = plugin.search(&client, &kw, &ext2).await;
            let _ = tx.send(result);
        });

        // 3. 等待响应或超时
        let wait = self.response_timeout;
        match tokio::time::timeout(wait, rx).await {
            Ok(Ok(Ok(results))) => {
                // 合并旧缓存
                let merged = self.merge_with_cache(&ck, results);
                self.store_cache(&ck, merged.clone(), true);
                self.update_main_cache(main_cache_key, &merged, true);
                PluginSearchResult {
                    results: merged, is_final: true, timestamp: Utc::now(),
                    source: self.plugin.name().to_string(), message: "搜索完成".into(),
                }
            }
            Ok(Ok(Err(e))) => {
                tracing::debug!("[{}] 搜索失败: {}", self.plugin.name(), e);
                PluginSearchResult { results: vec![], is_final: false, timestamp: Utc::now(),
                    source: self.plugin.name().to_string(), message: e.to_string() }
            }
            Ok(Err(_)) => {
                PluginSearchResult { results: vec![], is_final: false, timestamp: Utc::now(),
                    source: self.plugin.name().to_string(), message: "任务取消".into() }
            }
            Err(_) => {
                // 超时：返回部分缓存，后台继续。
                // 关键：不得用空结果覆盖已有缓存（否则并发挤兑时缓存被锁死为空，直到 TTL 过期）。
                let elapsed = deadline.elapsed();
                let partial = API_CACHE.get(&ck).map(|c| c.results).unwrap_or_default();
                if !partial.is_empty() {
                    self.store_cache(&ck, partial.clone(), false);
                }
                PluginSearchResult {
                    results: partial, is_final: false, timestamp: Utc::now(),
                    source: self.plugin.name().to_string(),
                    message: format!("处理中，后台继续... ({}ms)", elapsed.as_millis()),
                }
            }
        }
    }

    /// 后台刷新缓存
    fn spawn_refresh(&self, keyword: String, cache_key: String, ext: ExtMap, main_key: String, old: CachedResponse) {
        let plugin = self.plugin.clone();
        let client = self.background_client.clone();
        let pool_sem = POOL.semaphore.clone();
        let active = POOL.active.clone();
        let max_tasks = POOL.max_tasks;
        tokio::spawn(async move {
            if active.load(Ordering::Relaxed) >= max_tasks { return; }
            let _permit = match pool_sem.try_acquire_owned() { Ok(p) => p, Err(_) => return };
            active.fetch_add(1, Ordering::Relaxed);
            let _guard = ActiveGuard(active);
            match plugin.search(&client, &keyword, &ext).await {
                Ok(results) if !results.is_empty() => {
                    let mut seen: std::collections::HashSet<String> = results.iter().map(|r| r.unique_id.clone()).collect();
                    let mut merged = results;
                    for r in old.results {
                        if seen.insert(r.unique_id.clone()) { merged.push(r); }
                    }
                    API_CACHE.insert(cache_key, CachedResponse {
                        results: merged.clone(), timestamp: Utc::now().timestamp(), complete: true,
                        access_count: old.access_count,
                    });
                    // 更新主缓存
                    if !main_key.is_empty() {
                        cache::main_cache_set(main_key, serde_json::to_vec(&merged).unwrap_or_default());
                    }
                }
                _ => {}
            }
        });
    }

    fn merge_with_cache(&self, ck: &str, mut results: Vec<SearchResult>) -> Vec<SearchResult> {
        if let Some(old) = API_CACHE.get(ck) {
            if !old.results.is_empty() {
                let mut seen: std::collections::HashSet<String> = results.iter().map(|r| r.unique_id.clone()).collect();
                for r in old.results {
                    if seen.insert(r.unique_id.clone()) { results.push(r); }
                }
            }
        }
        results
    }

    fn store_cache(&self, ck: &str, results: Vec<SearchResult>, complete: bool) {
        let access = API_CACHE.get(ck).map(|c| c.access_count + 1).unwrap_or(1);
        API_CACHE.insert(ck.to_string(), CachedResponse {
            results, timestamp: Utc::now().timestamp(), complete, access_count: access,
        });
    }

    fn update_main_cache(&self, main_key: &str, results: &[SearchResult], is_final: bool) {
        if main_key.is_empty() || results.is_empty() || !is_final { return; }
        let hash = format!("{}_{}", results.len(), results.first().map(|r| r.unique_id.as_str()).unwrap_or(""));
        let key = format!("final_{}_{}_{}", self.plugin.name(), main_key, hash);
        {
            let mut t = self.final_tracker.lock().unwrap();
            if t.contains_key(&key) { return; }
            t.insert(key, true);
        }
        cache::main_cache_set(main_key.to_string(), serde_json::to_vec(results).unwrap_or_default());
    }
}

struct ActiveGuard(Arc<AtomicUsize>);
impl Drop for ActiveGuard {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::Relaxed); }
}
