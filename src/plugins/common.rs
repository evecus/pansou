//! 插件公共辅助：链接/密码提取、时间解析、去重、Flarum/APICMS 通用响应结构

use crate::model::{Link, SearchResult};
use crate::util::netdisk::{extract_netdisk_links, extract_password, get_link_type, clean_url};
use crate::util::text::{clean_text, matches_keywords};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"')\]，。]+"#).unwrap());

/// 从文本里提取第一个 URL
pub fn first_url(text: &str) -> String {
    URL_RE.find(text).map(|m| clean_url(m.as_str())).unwrap_or_default()
}

/// 从文本中提取所有网盘链接（带密码关联）
pub fn links_from_text(text: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for url in extract_netdisk_links(text) {
        let url = clean_url(&url);
        if url.is_empty() || !seen.insert(url.clone()) { continue; }
        let mut password = extract_password(text, &url);
        if password.is_empty() {
            // 行内查找 "提取码/密码: xxxx"
            if let Some(pos) = text.find(&url) {
                let ctx = &text[pos..(pos + url.len() + 40).min(text.len())];
                password = extract_password(ctx, &url);
            }
        }
        out.push(Link {
            r#type: get_link_type(&url).to_string(),
            url,
            password,
            datetime: None,
            work_title: None,
        });
    }
    out
}

/// 解析常见时间格式
pub fn parse_datetime(s: &str) -> DateTime<Utc> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) { return dt.with_timezone(&Utc); }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y/%m/%d %H:%M:%S", "%Y-%m-%d"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return DateTime::from_naive_utc_and_offset(dt, Utc);
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, fmt) {
            return DateTime::from_naive_utc_and_offset(d.and_hms_opt(0, 0, 0).unwrap(), Utc);
        }
    }
    // Unix 时间戳
    if let Ok(ts) = s.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp(ts, 0) { return dt; }
    }
    Utc::now()
}

/// 结果去重（按 unique_id）+ 关键词过滤 + 时间降序
pub fn finalize(mut results: Vec<SearchResult>, keyword: &str) -> Vec<SearchResult> {
    let mut seen = std::collections::HashSet::new();
    results.retain(|r| seen.insert(r.unique_id.clone()));
    results.retain(|r| matches_keywords(&r.title, &r.content, keyword));
    results.sort_by(|a, b| b.datetime.cmp(&a.datetime));
    results
}

/// 清理 HTML（Flarum 帖子等）
pub fn clean_html(html: &str) -> String {
    clean_text(html)
}

// ============ Flarum API 通用响应结构 ============

#[derive(Debug, Deserialize, Default)]
pub struct FlarumResponse {
    #[serde(default)]
    pub links: FlarumLinks,
    #[serde(default)]
    pub data: Vec<FlarumDiscussion>,
    #[serde(default)]
    pub included: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct FlarumLinks {
    #[serde(default)]
    pub next: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct FlarumDiscussion {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub attributes: FlarumDiscussionAttrs,
    #[serde(default)]
    pub relationships: FlarumRelationships,
}

#[derive(Debug, Deserialize, Default)]
pub struct FlarumDiscussionAttrs {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, rename = "createdAt")]
    pub created_at_camel: String,
}

impl FlarumDiscussionAttrs {
    pub fn created(&self) -> &str {
        if !self.created_at_camel.is_empty() { &self.created_at_camel } else { &self.created_at }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct FlarumRelationships {
    #[serde(default, rename = "mostRelevantPost")]
    pub most_relevant_post: Option<FlarumPostRef>,
    #[serde(default, rename = "firstPost")]
    pub first_post: Option<FlarumPostRef>,
}

#[derive(Debug, Deserialize, Default)]
pub struct FlarumPostRef {
    #[serde(default)]
    pub data: Option<FlarumPostData>,
}

#[derive(Debug, Deserialize, Default)]
pub struct FlarumPostData {
    #[serde(default)]
    pub id: String,
}

/// 从 Flarum included 中构建 post_id -> content_html 映射
pub fn flarum_post_map(included: &[serde_json::Value]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for item in included {
        let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "posts" { continue; }
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let html = item.get("attributes")
            .and_then(|a| a.get("contentHtml").or_else(|| a.get("content")))
            .and_then(|v| v.as_str()).unwrap_or("");
        if !id.is_empty() { map.insert(id, html.to_string()); }
    }
    map
}
