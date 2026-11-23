//! JSON/API 搜索类插件（已验证可用）
//! 当前：haisou（两阶段 JSON API）

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{clean_html, url_password};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashSet;

fn q(s: &str) -> String { crate::util::text::url_encode(s) }

// ============================================================
// haisou: https://haisou.cc/api/pan/share/search?query=&scope=title&pan=&page=
// code==0; data.list[] {hsid,platform,share_name,stat_file,stat_size}
// 第二阶段: /api/pan/share/{hsid}/fetch -> {share_code,share_pwd}
// 5 种网盘 × 2 页
// ============================================================

pub struct Haisou;
const HAISOU_TYPES: &[&str] = &["ali", "baidu", "quark", "xunlei", "tianyi"];

#[derive(Debug, Deserialize, Default)]
struct HaiSearchResp {
    #[serde(default)] code: i32,
    #[serde(default)] data: HaiSearchData,
}
#[derive(Debug, Deserialize, Default)]
struct HaiSearchData {
    #[serde(default)] list: Vec<HaiItem>,
}
#[derive(Debug, Deserialize, Default)]
struct HaiItem {
    #[serde(default)] hsid: String,
    #[serde(default)] platform: String,
    #[serde(default, rename = "share_name")] share_name: String,
    #[serde(default, rename = "stat_file")] stat_file: i64,
    #[serde(default, rename = "stat_size")] stat_size: i64,
}
#[derive(Debug, Deserialize, Default)]
struct HaiFetchResp {
    #[serde(default)] code: i32,
    #[serde(default)] data: HaiFetchData,
}
#[derive(Debug, Deserialize, Default)]
struct HaiFetchData {
    #[serde(default, rename = "share_code")] share_code: String,
    #[serde(default, rename = "share_pwd")] share_pwd: Option<String>,
}

fn hai_url(platform: &str, code: &str) -> Option<(String, &'static str)> {
    let (prefix, ty) = match platform.to_lowercase().as_str() {
        "ali" => ("https://www.alipan.com/s/", "aliyun"),
        "baidu" => ("https://pan.baidu.com/s/", "baidu"),
        "quark" => ("https://pan.quark.cn/s/", "quark"),
        "xunlei" => ("https://pan.xunlei.com/s/", "xunlei"),
        "tianyi" => ("https://cloud.189.cn/t/", "tianyi"),
        _ => return None,
    };
    Some((format!("{}{}", prefix, code), ty))
}

fn fmt_size(size: i64) -> String {
    const GB: i64 = 1024 * 1024 * 1024;
    const MB: i64 = 1024 * 1024;
    if size >= GB { format!("{:.2} GB", size as f64 / GB as f64) }
    else if size >= MB { format!("{:.2} MB", size as f64 / MB as f64) }
    else { format!("{} B", size) }
}

#[async_trait]
impl SearchPlugin for Haisou {
    fn name(&self) -> &'static str { "haisou" }
    fn priority(&self) -> i32 { 3 }

    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        // 阶段1：并发搜索所有网盘类型 × 2 页
        let mut futs = Vec::new();
        for pt in HAISOU_TYPES {
            for page in 1..=2u32 {
                let url = format!(
                    "https://haisou.cc/api/pan/share/search?query={}&scope=title&pan={}&page={}&filter_valid=true&filter_has_files=false",
                    q(keyword), pt, page
                );
                let c = client.clone();
                futs.push(async move { c.get_text(&url, Some(browser_headers(Some("https://haisou.cc/")))).await });
            }
        }
        let responses = futures::future::join_all(futs).await;
        let mut items: Vec<HaiItem> = Vec::new();
        for body in responses.into_iter().flatten() {
            let resp: HaiSearchResp = serde_json::from_str(&body).unwrap_or_default();
            if resp.code != 0 { continue; }
            items.extend(resp.data.list);
        }
        // 去重 hsid
        let mut seen = HashSet::new();
        items.retain(|it| seen.insert(it.hsid.clone()));
        if items.is_empty() { return Ok(Vec::new()); }

        // 阶段2：并发 fetch 每个 hsid 获取真实分享码
        let mut f2 = Vec::new();
        for it in &items {
            let url = format!("https://haisou.cc/api/pan/share/{}/fetch", it.hsid);
            let c = client.clone();
            let hsid = it.hsid.clone();
            let platform = it.platform.clone();
            f2.push(async move {
                let body = c.get_text(&url, Some(browser_headers(Some("https://haisou.cc/")))).await.ok();
                (hsid, platform, body)
            });
        }
        let fetched = futures::future::join_all(f2).await;
        let mut out = Vec::new();
        // items 与 fetched 通过 join_all 保序一一对应
        for (it, (_hsid, _platform, body)) in items.into_iter().zip(fetched.into_iter()) {
            let body = match body { Some(b) => b, None => continue };
            let fr: HaiFetchResp = serde_json::from_str(&body).unwrap_or_default();
            if fr.code != 0 { continue; }
            let (url, ty) = match hai_url(&it.platform, &fr.data.share_code) { Some(x) => x, None => continue };
            let mut r = SearchResult {
                unique_id: format!("haisou-{}", it.hsid),
                title: clean_html(&it.share_name),
                datetime: Utc::now(),
                channel: String::new(),
                ..Default::default()
            };
            if r.title.is_empty() { r.title = "未知资源".to_string(); }
            r.content = format!("文件数量: {} | 网盘类型: {} | 大小: {}", it.stat_file, it.platform, fmt_size(it.stat_size));
            r.tags.push(it.platform.clone());
            let mut pass = fr.data.share_pwd.unwrap_or_default();
            if pass.is_empty() { pass = url_password(&url); }
            r.links.push(Link {
                r#type: ty.to_string(), url, password: pass, datetime: None, work_title: None,
            });
            out.push(r);
        }
        Ok(out)
    }
}
