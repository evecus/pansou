//! 内存缓存系统 - 用 moka 实现 TTL 缓存

use moka::sync::Cache as MokaCache;
use once_cell::sync::Lazy;
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

/// API 响应缓存（keyword/plugin 维度）
struct TypedCache<T>(MokaCache<String, T>);

impl<T: Clone + Send + Sync + 'static> TypedCache<T> {
    fn new(ttl: Duration, max_capacity: u64) -> Self {
        Self(MokaCache::builder()
            .time_to_live(ttl)
            .max_capacity(max_capacity)
            .build())
    }
    fn get(&self, k: &str) -> Option<T> { self.0.get(&k.to_string()) }
    fn set(&self, k: String, v: T) { self.0.insert(k, v); }
    fn invalidate(&self, k: &str) { self.0.invalidate(&k.to_string()); }
}

/// 通用 JSON 字节缓存（用于主搜索结果缓存）
pub static MAIN_CACHE: Lazy<MokaCache<String, Vec<u8>>> = Lazy::new(|| {
    MokaCache::builder()
        .time_to_live(Duration::from_secs(crate::config::get().cache_ttl_minutes * 60))
        .max_capacity(10_000)
        .build()
});

pub fn main_cache_get(key: &str) -> Option<Vec<u8>> { MAIN_CACHE.get(&key.to_string()) }
pub fn main_cache_set(key: String, data: Vec<u8>) { MAIN_CACHE.insert(key, data); }

/// 通用 JSON 对象缓存辅助
pub fn cache_get_json<T: DeserializeOwned>(key: &str) -> Option<T> {
    let raw = MAIN_CACHE.get(&key.to_string())?;
    serde_json::from_slice(&raw).ok()
}
pub fn cache_set_json<T: Serialize>(key: String, value: &T) {
    if let Ok(raw) = serde_json::to_vec(value) {
        MAIN_CACHE.insert(key, raw);
    }
}
