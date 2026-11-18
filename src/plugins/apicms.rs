//! Apple CMS vod API 通用插件（wanou / ouge）
//! 响应: {code, list:[{vod_id, vod_name, vod_down_from, vod_down_url, vod_content, ...}]}
//! 链接: vod_down_from 与 vod_down_url 按 $$$ 分隔一一对应。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{determine_type, parse_time, url_password};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

pub struct ApicmsPlugin {
    pub name: &'static str,
    pub api_url: &'static str, // 形如 https://host/api.php/provide/vod?ac=detail&wd={KW}
    pub referer: &'static str,
}

#[derive(Debug, Deserialize, Default)]
struct ApiResponse {
    #[serde(default)]
    code: i32,
    #[serde(default)]
    list: Vec<Value>,
}

fn map_type(api_type: &str, url: &str) -> String {
    let t = api_type.trim().to_uppercase();
    let by_api = match t.as_str() {
        "BD" => Some("baidu"), "KG" => Some("quark"), "UC" => Some("uc"),
        "ALY" => Some("aliyun"), "XL" => Some("xunlei"), "TY" => Some("tianyi"),
        "115" => Some("115"), "123" => Some("123"), "MB" | "MOBILE" | "CM" => Some("mobile"),
        "PIKPAK" => Some("pikpak"),
        _ => None,
    };
    if let Some(x) = by_api { return x.to_string(); }
    determine_type(url).to_string()
}

#[async_trait]
impl SearchPlugin for ApicmsPlugin {
    fn name(&self) -> &'static str { self.name }

    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = self.api_url.replace("{KW}", &crate::util::text::url_encode(keyword));
        let (_, body, _) = client.request_text(reqwest::Method::GET, &url,
            Some(browser_headers(Some(self.referer))), None).await?;
        let resp: ApiResponse = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("[{}] JSON 解析失败: {}", self.name, e);
                return Ok(Vec::new());
            }
        };
        // Go 版要求 code == 1，否则视为 API 错误
        if resp.code != 1 {
            tracing::debug!("[{}] API 返回错误 code={}", self.name, resp.code);
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for item in resp.list {
            let name = item.get("vod_name").and_then(|x| x.as_str()).unwrap_or("");
            let froms = item.get("vod_down_from").and_then(|x| x.as_str()).unwrap_or("");
            let urls = item.get("vod_down_url").and_then(|x| x.as_str()).unwrap_or("");
            if froms.is_empty() || urls.is_empty() { continue; }
            let fp: Vec<&str> = froms.split("$$$").collect();
            let up: Vec<&str> = urls.split("$$$").collect();
            let mut links = Vec::new();
            for i in 0..fp.len().min(up.len()) {
                let u = up[i].trim();
                if u.is_empty() || u.contains("javascript:") || u.contains('#') { continue; }
                let ty = map_type(fp[i], u);
                if ty == "others" && !u.starts_with("magnet:") && !u.starts_with("ed2k:") { continue; }
                if links.iter().any(|l: &Link| l.url == u) { continue; }
                links.push(Link {
                    r#type: ty,
                    url: u.to_string(),
                    password: url_password(u),
                    datetime: None,
                    work_title: None,
                });
            }
            if links.is_empty() { continue; }
            let content = item.get("vod_content").and_then(|x| x.as_str()).unwrap_or("");
            let actor = item.get("vod_actor").and_then(|x| x.as_str()).unwrap_or("");
            let director = item.get("vod_director").and_then(|x| x.as_str()).unwrap_or("");
            let remarks = item.get("vod_remarks").and_then(|x| x.as_str()).unwrap_or("");
            let content = format!("{}\n导演：{}\n主演：{}\n{}", content, director, actor, remarks);
            let id = crate::plugins::helpers::json_id(item.get("vod_id"));
            let date = item.get("vod_pubdate").and_then(|x| x.as_str()).unwrap_or("");
            out.push(SearchResult {
                unique_id: format!("{}-{}", self.name, id),
                title: name.to_string(),
                content,
                datetime: if date.is_empty() { Utc::now() } else { parse_time(date) },
                links,
                ..Default::default()
            });
        }
        Ok(out)
    }
}
