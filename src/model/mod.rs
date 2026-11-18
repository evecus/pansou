//! 数据模型定义 - 与 Go 版本 model 包保持 JSON 兼容

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 网盘链接
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Link {
    pub r#type: String,
    pub url: String,
    #[serde(default)]
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_title: Option<String>,
}

/// 搜索结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResult {
    #[serde(default)]
    pub message_id: String,
    pub unique_id: String,
    #[serde(default)]
    pub channel: String,
    pub datetime: DateTime<Utc>,
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// 合并后的网盘链接
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MergedLink {
    pub url: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub note: String,
    pub datetime: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// 按网盘类型分组的合并链接
pub type MergedLinks = std::collections::HashMap<String, Vec<MergedLink>>;

/// 搜索响应
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResponse {
    pub total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<SearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_by_type: Option<MergedLinks>,
}

/// 过滤配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FilterConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

/// 搜索请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub kw: String,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub conc: i32,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub res: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub plugins: Option<Vec<String>>,
    #[serde(default)]
    pub ext: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub cloud_types: Option<Vec<String>>,
    #[serde(default)]
    pub filter: Option<FilterConfig>,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            kw: String::new(),
            channels: vec![],
            conc: 0,
            refresh: false,
            res: String::new(),
            src: String::new(),
            plugins: None,
            ext: serde_json::Map::new(),
            cloud_types: None,
            filter: None,
        }
    }
}

/// 通用 API 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response<T> {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T> Response<T> {
    pub fn success(data: T) -> Self {
        Self { code: 0, message: "success".into(), data: Some(data) }
    }
    pub fn error(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
}

/// 插件搜索结果（带 IsFinal 标记）
#[derive(Debug, Clone, Default)]
pub struct PluginSearchResult {
    pub results: Vec<SearchResult>,
    pub is_final: bool,
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub message: String,
}

/// 链接检测相关模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub disk_type: String,
    pub url: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckRequest {
    pub items: Vec<CheckItem>,
    #[serde(default)]
    pub view_token: String,
    #[serde(default)]
    pub proxy: String,
    #[serde(default)]
    pub proxy_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckResult {
    pub disk_type: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_url: Option<String>,
    pub state: String,
    pub cache_hit: bool,
    pub checked_at: i64,
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckResponse {
    pub results: Vec<CheckResult>,
}
