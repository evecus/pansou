//! 配置系统 - 与 Go 版本保持相同的环境变量和默认值

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// 默认 Telegram 频道列表
pub const DEFAULT_CHANNELS: &[&str] = &[
    "tgsearchers4", "Aliyun_4K_Movies", "bdbdndn11", "yunpanx", "bsbdbfjfjff", "yp123pan",
    "sbsbsnsqq", "yunpanxunlei", "tianyifc", "BaiduCloudDisk", "txtyzy", "peccxinpd",
    "gotopan", "PanjClub", "kkxlzy", "baicaoZY", "MCPH01", "MCPH02",
    "MCPH03", "bdwpzhpd", "ysxb48", "jdjdn1111", "yggpan", "MCPH086",
    "zaihuayun", "Q66Share", "ucwpzy", "shareAliyun", "alyp_1", "dianyingshare",
    "Quark_Movies", "XiangxiuNBB", "ydypzyfx", "ucquark", "xx123pan", "yingshifenxiang123",
    "zyfb123", "tyypzhpd", "tianyirigeng", "cloudtianyi", "hdhhd21", "Lsp115",
    "oneonefivewpfx", "qixingzhenren", "taoxgzy", "Channel_Shares_115", "tyysypzypd", "vip115hot",
    "wp123zy", "yunpan139", "yunpan189", "yunpanuc", "yydf_hzl", "leoziyuan",
    "Q_dongman", "yoyokuakeduanju", "TG654TG", "WFYSFX02", "QukanMovie", "yeqingjie_GJG666",
    "movielover8888_film3", "Baidu_netdisk", "D_wusun", "FLMdongtianfudi", "KaiPanshare", "QQZYDAPP",
    "rjyxfx", "PikPak_Share_Channel", "btzhi", "newproductsourcing", "cctv1211", "duan_ju",
    "QuarkFree", "yunpanNB", "kkdj001", "xxzlzn", "pxyunpanxunlei", "jxwpzy",
    "kuakedongman", "liangxingzhinan", "xiangnikanj", "solidsexydoll", "guoman4K", "zdqxm",
    "kduanju", "cilidianying", "CBduanju", "SharePanFilms", "dzsgx", "BooksRealm",
    "Oscar_4Kmovies", "douerpan", "baidu_yppan", "Q_jilupian", "Netdisk_Movies", "yunpanquark",
    "ammmziyuan", "ciliziyuanku", "cili8888", "jzmm_123pan", "Q_dianying", "domgmingapk",
    "dianying4k", "q_dianshiju", "tgbokee", "ucshare", "godupan", "gokuapan",
];

#[derive(Debug, Clone)]
pub struct Config {
    pub default_channels: Vec<String>,
    pub default_concurrency: usize,
    pub port: String,
    pub proxy_url: String,
    pub http_proxy_url: String,
    pub https_proxy_url: String,

    pub cache_enabled: bool,
    pub cache_path: PathBuf,
    pub cache_max_size_mb: usize,
    pub cache_ttl_minutes: u64,

    pub enable_compression: bool,
    pub min_size_to_compress: usize,

    pub plugin_timeout: Duration,

    pub async_plugin_enabled: bool,
    pub enabled_plugins: Option<Vec<String>>,
    pub async_response_timeout: Duration,
    pub async_max_background_workers: usize,
    pub async_max_background_tasks: usize,
    pub async_cache_ttl_hours: u64,
    pub async_log_enabled: bool,

    pub http_read_timeout: Duration,
    pub http_write_timeout: Duration,
    pub http_idle_timeout: Duration,
    pub http_max_conns: usize,

    pub auth_enabled: bool,
    pub auth_users: HashMap<String, String>,
    pub auth_token_expiry: Duration,
    pub auth_jwt_secret: String,
}

fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}
fn env_bool_loose(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) => v != "false" && v != "0",
        Err(_) => default,
    }
}
fn env_i64(key: &str, default: i64) -> i64 {
    env::var(key).ok().and_then(|v| v.parse().ok()).filter(|v| *v > 0).unwrap_or(default)
}

impl Config {
    pub fn load() -> Self {
        let proxy_url = env_str("PROXY", "");
        let http_proxy = {
            let v = env::var("HTTP_PROXY").unwrap_or_default();
            if v.is_empty() { env::var("http_proxy").unwrap_or_default() } else { v }
        };
        let https_proxy = {
            let v = env::var("HTTPS_PROXY").unwrap_or_default();
            if v.is_empty() { env::var("https_proxy").unwrap_or_default() } else { v }
        };

        let default_channels: Vec<String> = match env::var("CHANNELS") {
            Ok(v) if !v.is_empty() => v.split(',').map(|s| s.to_string()).collect(),
            _ => DEFAULT_CHANNELS.iter().map(|s| s.to_string()).collect(),
        };

        let plugin_timeout_secs = env_i64("PLUGIN_TIMEOUT", 30);
        let async_resp_secs = env_i64("ASYNC_RESPONSE_TIMEOUT", 4);

        let async_plugin_enabled = env_bool_loose("ASYNC_PLUGIN_ENABLED", true);
        let enabled_plugins = match env::var("ENABLED_PLUGINS") {
            Ok(v) if v.is_empty() => Some(vec![]),
            Ok(v) => Some(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
            Err(_) => None,
        };

        let cpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let max_workers = {
            let explicit = env::var("ASYNC_MAX_BACKGROUND_WORKERS").ok().and_then(|v| v.parse().ok()).filter(|v: &usize| *v > 0);
            explicit.unwrap_or_else(|| (cpu * 5).max(20))
        };
        let max_tasks = {
            let explicit = env::var("ASYNC_MAX_BACKGROUND_TASKS").ok().and_then(|v| v.parse().ok()).filter(|v: &usize| *v > 0);
            explicit.unwrap_or_else(|| (max_workers * 5).max(100))
        };

        let read_timeout = {
            let explicit = env::var("HTTP_READ_TIMEOUT").ok().and_then(|v| v.parse::<i64>().ok()).filter(|v| *v > 0);
            match explicit {
                Some(s) => Duration::from_secs(s as u64),
                None => {
                    let base = 30u64;
                    Duration::from_secs(if async_plugin_enabled {
                        base.max(async_resp_secs as u64 * 3)
                    } else { base })
                }
            }
        };
        let write_timeout_secs = {
            let explicit = env::var("HTTP_WRITE_TIMEOUT").ok().and_then(|v| v.parse::<i64>().ok()).filter(|v| *v > 0);
            match explicit {
                Some(s) => s as u64,
                None => 60u64.max(plugin_timeout_secs as u64 * 3 / 2),
            }
        };
        let write_timeout = Duration::from_secs(write_timeout_secs);
        let idle_timeout = Duration::from_secs(
            env::var("HTTP_IDLE_TIMEOUT").ok().and_then(|v| v.parse::<i64>().ok()).filter(|v| *v > 0).unwrap_or(120) as u64
        );
        let max_conns = env::var("HTTP_MAX_CONNS").ok().and_then(|v| v.parse().ok()).filter(|v: &usize| *v > 0).unwrap_or_else(|| (cpu * 200).max(1000));

        let mut default_concurrency = env::var("CONCURRENCY").ok().and_then(|v| v.parse().ok()).filter(|v: &usize| *v > 0)
            .unwrap_or_else(|| default_channels.len() + 7 + 10);

        let auth_enabled = matches!(env::var("AUTH_ENABLED").as_deref(), Ok("true") | Ok("1"));
        let auth_users: HashMap<String, String> = env::var("AUTH_USERS").map(|v| {
            v.split(',').filter_map(|pair| {
                let mut it = pair.splitn(2, ':');
                let u = it.next()?.trim();
                let p = it.next()?.trim();
                if u.is_empty() || p.is_empty() { None } else { Some((u.to_string(), p.to_string())) }
            }).collect()
        }).unwrap_or_default();
        let auth_token_expiry = Duration::from_secs(env_i64("AUTH_TOKEN_EXPIRY", 24) as u64 * 3600);
        let auth_jwt_secret = env::var("AUTH_JWT_SECRET").unwrap_or_else(|_| {
            format!("pansou-default-secret-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
        });

        let cache_path = env::var("CACHE_PATH").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("pansou-cache"));

        // 并发数会根据实际插件数在启动后更新
        default_concurrency = default_concurrency.max(1);

        Self {
            default_channels,
            default_concurrency,
            port: env_str("PORT", "4041"),
            proxy_url,
            http_proxy_url: http_proxy,
            https_proxy_url: https_proxy,
            cache_enabled: env_bool_loose("CACHE_ENABLED", true),
            cache_path,
            cache_max_size_mb: env_i64("CACHE_MAX_SIZE", 100) as usize,
            cache_ttl_minutes: env_i64("CACHE_TTL", 60) as u64,
            enable_compression: matches!(env::var("ENABLE_COMPRESSION").as_deref(), Ok("true") | Ok("1")),
            min_size_to_compress: env_i64("MIN_SIZE_TO_COMPRESS", 1024) as usize,
            plugin_timeout: Duration::from_secs(plugin_timeout_secs as u64),
            async_plugin_enabled,
            enabled_plugins,
            async_response_timeout: Duration::from_secs(async_resp_secs as u64),
            async_max_background_workers: max_workers,
            async_max_background_tasks: max_tasks,
            async_cache_ttl_hours: env_i64("ASYNC_CACHE_TTL_HOURS", 1) as u64,
            async_log_enabled: env_bool_loose("ASYNC_LOG_ENABLED", true),
            http_read_timeout: read_timeout,
            http_write_timeout: write_timeout,
            http_idle_timeout: idle_timeout,
            http_max_conns: max_conns,
            auth_enabled,
            auth_users,
            auth_token_expiry,
            auth_jwt_secret,
        }
    }

    /// 根据实际插件数更新默认并发数（未通过 CONCURRENCY 指定时）
    pub fn update_concurrency(&mut self, plugin_count: usize) {
        if env::var("CONCURRENCY").is_ok() { return; }
        self.default_concurrency = (self.default_channels.len() + plugin_count + 10).max(1);
    }
}

static CONFIG: OnceLock<Config> = OnceLock::new();

pub fn init() -> &'static Config {
    CONFIG.get_or_init(Config::load)
}

pub fn get() -> &'static Config {
    CONFIG.get_or_init(Config::load)
}
