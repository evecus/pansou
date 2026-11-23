//! JSON API 插件（已验证可用）：hunhepan（4 镜像 × 3 页 POST JSON）

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::parse_time;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Deserialize;

/// 4 个镜像站： (api_url, referer, 额外 Origin)
const APIS: &[(&str, &str, Option<&str>)] = &[
    ("https://hunhepan.com/open/search/disk", "https://hunhepan.com/search", None),
    ("https://qkpanso.com/v1/search/disk", "https://qkpanso.com/search", None),
    ("https://kuake8.com/v1/search/disk", "https://kuake8.com/search", None),
    ("https://www.misoso.cc/v1/search/disk", "https://www.misoso.cc/search", Some("https://www.misoso.cc")),
];

#[derive(Debug, Deserialize, Default)]
struct HhpResp {
    #[serde(default)] code: i32,
    #[serde(default)] data: HhpData,
}
#[derive(Debug, Deserialize, Default)]
struct HhpData {
    #[serde(default)] list: Vec<HhpItem>,
}
#[derive(Debug, Deserialize, Default)]
struct HhpItem {
    #[serde(default, rename = "disk_id")] disk_id: String,
    #[serde(default, rename = "disk_name")] disk_name: String,
    #[serde(default, rename = "disk_pass")] disk_pass: String,
    #[serde(default, rename = "disk_type")] disk_type: String,
    #[serde(default)] files: String,
    #[serde(default, rename = "shared_time")] shared_time: String,
    #[serde(default)] link: String,
}

fn hhp_type(t: &str) -> &'static str {
    match t {
        "BDY" => "baidu", "ALY" => "aliyun", "QUARK" => "quark", "TIANYI" => "tianyi",
        "UC" => "uc", "CAIYUN" => "mobile", "115" => "115", "XUNLEI" => "xunlei",
        "123PAN" => "123", "PIKPAK" => "pikpak", _ => "others",
    }
}

fn clean_title(s: &str) -> String {
    s.replace("<em>", "").replace("</em>", "").replace("<b>", "").replace("</b>", "")
        .replace("<strong>", "").replace("</strong>", "").replace("<i>", "").replace("</i>", "")
        .trim().to_string()
}

pub struct Hunhepan;

#[async_trait]
impl SearchPlugin for Hunhepan {
    fn name(&self) -> &'static str { "hunhepan" }
    fn priority(&self) -> i32 { 3 }

    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let mut futs = Vec::new();
        for (api, referer, origin) in APIS {
            for page in 1..=3u32 {
                let body = serde_json::json!({
                    "page": page, "q": keyword, "user": "", "exact": false,
                    "format": [], "share_time": "", "size": 30, "type": "",
                    "exclude_user": [], "adv_params": {"wechat_pwd": "", "platform": "pc"}
                }).to_string();
                let mut h = HeaderMap::new();
                h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                h.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
                if let Ok(v) = HeaderValue::from_str(referer) { h.insert("referer", v); }
                if let Some(o) = origin { if let Ok(v) = HeaderValue::from_str(o) { h.insert("origin", v); } }
                let c = client.clone();
                let api = *api;
                futs.push(async move {
                    c.request_text(reqwest::Method::POST, api, Some(h), Some(body)).await
                        .ok().map(|(_, text, _)| text)
                });
            }
        }
        let responses = futures::future::join_all(futs).await;
        let mut items: Vec<HhpItem> = Vec::new();
        for text in responses.into_iter().flatten() {
            let resp: HhpResp = serde_json::from_str(&text).unwrap_or_default();
            if resp.code != 200 { continue; }
            items.extend(resp.data.list);
        }
        // 去重（disk_id 优先）
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut idx = 0;
        for it in items {
            if it.link.is_empty() { continue; }
            let key = if !it.disk_id.is_empty() { it.disk_id.clone() }
                else { format!("{}|{}", it.link, clean_title(&it.disk_name)) };
            if !seen.insert(key.clone()) { continue; }
            let uid = if !it.disk_id.is_empty() { it.disk_id.clone() }
                else { format!("{}-{}", Utc::now().timestamp(), idx) };
            idx += 1;
            let mut r = SearchResult {
                unique_id: format!("hunhepan-{}", uid),
                title: clean_title(&it.disk_name),
                content: it.files.clone(),
                datetime: if it.shared_time.is_empty() { Utc::now() } else { parse_time(&it.shared_time) },
                channel: String::new(),
                ..Default::default()
            };
            let mut pass = it.disk_pass.clone();
            if pass.is_empty() { pass = crate::plugins::helpers::url_password(&it.link); }
            r.links.push(Link {
                r#type: hhp_type(&it.disk_type).to_string(),
                url: it.link.clone(), password: pass, datetime: None, work_title: None,
            });
            out.push(r);
        }
        Ok(out)
    }
}
