//! 缓存系统：内存（moka）或磁盘二选一。
//!
//! 环境变量：
//!  - CACHE_DISK_PATH：配置后缓存只存磁盘（不再存内存），重启后仍可命中；目录不可用时回退内存。
//!  - CACHE_MEMORY_ENABLED：是否存内存，默认 true；设为 false 且未配置磁盘路径 = 完全不缓存。
//!  - CACHE_CLEAN_INTERVAL：磁盘缓存定期清理间隔，支持 s/m/h/d 后缀（如 6h、1d），纯数字按分钟，默认 10 分钟。
//!  - CACHE_TTL：缓存有效期，同样支持 s/m/h/d 后缀，纯数字按分钟，默认 60 分钟。
//!  - CACHE_MAX_SIZE：磁盘缓存总大小上限（MB，默认 100），超出时按修改时间从旧到新删除。
//!
//! 磁盘文件格式：前 8 字节为过期时间（Unix 秒，小端），其后为数据。
//! 写入先写临时文件再 rename，读到半截文件的情况不会出现。

use crate::config;
use moka::sync::Cache as MokaCache;
use once_cell::sync::Lazy;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// 带 TTL 的字节缓存：后端为内存、磁盘或关闭。
pub struct KvCache {
    ttl: Duration,
    mem: Option<MokaCache<String, Vec<u8>>>,
    disk: Option<PathBuf>,
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

impl KvCache {
    /// name：磁盘模式下的子目录名；max_capacity：内存模式的条数上限。
    pub fn new(name: &str, ttl: Duration, max_capacity: u64) -> Self {
        let cfg = config::get();
        if let Some(root) = &cfg.cache_disk_path {
            let dir = root.join(name);
            match fs::create_dir_all(&dir) {
                Ok(_) => return Self { ttl, mem: None, disk: Some(dir) },
                Err(e) => tracing::warn!("[缓存] 磁盘目录 {:?} 不可用（{}），{}", dir, e,
                    if cfg.cache_memory_enabled { "回退为内存缓存" } else { "且内存缓存已关闭，缓存不生效" }),
            }
        }
        let mem = if cfg.cache_memory_enabled {
            Some(MokaCache::builder().time_to_live(ttl).max_capacity(max_capacity).build())
        } else { None };
        Self { ttl, mem, disk: None }
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        if let Some(m) = &self.mem { return m.get(&key.to_string()); }
        let dir = self.disk.as_ref()?;
        let path = file_path(dir, key);
        let raw = fs::read(&path).ok()?;
        if raw.len() < 8 { let _ = fs::remove_file(&path); return None; }
        let exp = u64::from_le_bytes(raw[..8].try_into().ok()?);
        if exp <= now_secs() { let _ = fs::remove_file(&path); return None; }
        Some(raw[8..].to_vec())
    }

    pub fn set(&self, key: &str, data: Vec<u8>) {
        if let Some(m) = &self.mem { m.insert(key.to_string(), data); return; }
        let Some(dir) = self.disk.as_ref() else { return };
        let path = file_path(dir, key);
        if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
        let tmp = path.with_extension(format!("tmp{}_{}", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
        let mut buf = Vec::with_capacity(data.len() + 8);
        buf.extend_from_slice(&(now_secs() + self.ttl.as_secs()).to_le_bytes());
        buf.extend_from_slice(&data);
        if let Err(e) = fs::write(&tmp, &buf).and_then(|_| fs::rename(&tmp, &path)) {
            tracing::warn!("[缓存] 写磁盘失败 {:?}: {}", path, e);
            let _ = fs::remove_file(&tmp);
        }
    }

    pub fn get_json<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_slice(&self.get(key)?).ok()
    }
    pub fn set_json<T: Serialize>(&self, key: &str, value: &T) {
        if let Ok(raw) = serde_json::to_vec(value) { self.set(key, raw); }
    }
}

/// 文件路径：<dir>/<sha256 前 2 位>/<sha256>.bin（按前缀分目录，避免单目录文件过多）
fn file_path(dir: &Path, key: &str) -> PathBuf {
    let hex = hex::encode(Sha256::digest(key.as_bytes()));
    dir.join(&hex[..2]).join(format!("{}.bin", hex))
}

/// 主搜索结果缓存（TG 频道结果等）
pub static MAIN_CACHE: Lazy<KvCache> = Lazy::new(|| {
    KvCache::new("main", Duration::from_secs(config::get().cache_ttl_minutes * 60), 10_000)
});

pub fn main_cache_get(key: &str) -> Option<Vec<u8>> { MAIN_CACHE.get(key) }
pub fn main_cache_set(key: String, data: Vec<u8>) { MAIN_CACHE.set(&key, data); }

/// 通用 JSON 对象缓存辅助
#[allow(dead_code)]
pub fn cache_get_json<T: DeserializeOwned>(key: &str) -> Option<T> { MAIN_CACHE.get_json(key) }
#[allow(dead_code)]
pub fn cache_set_json<T: Serialize>(key: String, value: &T) { MAIN_CACHE.set_json(&key, value); }

// ================= 磁盘清理 =================

/// 启动磁盘缓存清理：启动时先清一次（清掉上次运行遗留的过期文件），之后按间隔定期清理。
/// 未配置 CACHE_DISK_PATH 时什么都不做。
pub fn start_cleaner() {
    let cfg = config::get();
    match &cfg.cache_disk_path {
        Some(root) => tracing::info!("[缓存] 使用磁盘缓存 {:?}（不使用内存），清理间隔 {:?}，容量上限 {}MB",
            root, cfg.cache_clean_interval, cfg.cache_max_size_mb),
        None if cfg.cache_memory_enabled => tracing::info!("[缓存] 使用内存缓存"),
        None => tracing::warn!("[缓存] 内存缓存已关闭且未配置磁盘路径：缓存不生效"),
    }
    let Some(root) = cfg.cache_disk_path.clone() else { return };
    if let Err(e) = fs::create_dir_all(&root) {
        tracing::warn!("[缓存] 磁盘缓存目录 {:?} 创建失败：{}（各缓存将回退/失效）", root, e);
    }
    let interval = cfg.cache_clean_interval;
    let max_bytes = cfg.cache_max_size_mb as u64 * 1024 * 1024;
    tokio::spawn(async move {
        loop {
            let r = root.clone();
            let _ = tokio::task::spawn_blocking(move || clean_once(&r, max_bytes)).await;
            tokio::time::sleep(interval).await;
        }
    });
}

/// 一次清理：删过期文件、残留临时文件，总量超限时按修改时间从旧到新删到上限的 90%。
fn clean_once(root: &Path, max_bytes: u64) {
    let now = now_secs();
    let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    let mut removed = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for ent in rd.flatten() {
            let path = ent.path();
            let Ok(meta) = ent.metadata() else { continue };
            if meta.is_dir() { stack.push(path); continue; }
            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            let is_bin = path.extension().map(|e| e == "bin").unwrap_or(false);
            let expired = if is_bin {
                // 只读头 8 字节判断是否过期
                use std::io::Read;
                let mut hdr = [0u8; 8];
                match fs::File::open(&path).and_then(|mut f| f.read_exact(&mut hdr)) {
                    Ok(_) => u64::from_le_bytes(hdr) <= now,
                    Err(_) => true, // 损坏文件
                }
            } else {
                // 写入中断遗留的临时文件：超过 10 分钟视为垃圾
                mtime.elapsed().map(|d| d > Duration::from_secs(600)).unwrap_or(false)
            };
            if expired {
                if fs::remove_file(&path).is_ok() { removed += 1; }
            } else {
                files.push((path, meta.len(), mtime));
            }
        }
    }
    let total: u64 = files.iter().map(|f| f.1).sum();
    let mut evicted = 0usize;
    if max_bytes > 0 && total > max_bytes {
        files.sort_by_key(|f| f.2);
        let target = max_bytes / 10 * 9;
        let mut cur = total;
        for (p, len, _) in &files {
            if cur <= target { break; }
            if fs::remove_file(p).is_ok() { cur -= len; evicted += 1; }
        }
    }
    if removed > 0 || evicted > 0 {
        tracing::info!("[缓存] 磁盘清理：删除过期/残留 {} 个，超限淘汰 {} 个", removed, evicted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(dir: &Path, ttl: u64) -> KvCache {
        fs::create_dir_all(dir).unwrap();
        KvCache { ttl: Duration::from_secs(ttl), mem: None, disk: Some(dir.to_path_buf()) }
    }

    #[test]
    fn disk_roundtrip_and_expiry() {
        let root = std::env::temp_dir().join(format!("pansou-cache-test-{}", std::process::id()));
        let c = disk(&root.join("a"), 1);
        c.set("爱情公寓", b"hello".to_vec());
        assert_eq!(c.get("爱情公寓").unwrap(), b"hello");
        assert!(c.get("other").is_none());
        std::thread::sleep(Duration::from_millis(2100));
        assert!(c.get("爱情公寓").is_none(), "过期后应读不到");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cleaner_removes_expired_and_evicts() {
        let root = std::env::temp_dir().join(format!("pansou-clean-test-{}", std::process::id()));
        let short = disk(&root.join("s"), 1);
        let long = disk(&root.join("l"), 3600);
        short.set("x", vec![1; 10]);
        for i in 0..10 { long.set(&format!("k{}", i), vec![0; 1000]); }
        std::thread::sleep(Duration::from_millis(2100));
        clean_once(&root, 0);
        assert!(short.get("x").is_none());
        assert!(long.get("k0").is_some());
        // 容量上限 5000 字节：10 个 ~1008 字节的文件应被淘汰到 <= 4500
        clean_once(&root, 5000);
        let left = (0..10).filter(|i| long.get(&format!("k{}", i)).is_some()).count();
        assert!(left >= 1 && left <= 4, "left={}", left);
        let _ = fs::remove_dir_all(&root);
    }
}
