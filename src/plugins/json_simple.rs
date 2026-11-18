//! JSON/API 类插件（简单组）：jikepan / meitizy / melost / yunsou / ash

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::*;
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::header::HeaderValue;
use serde_json::{json, Value};

fn r(n: &str, title: &str, content: &str, links: Vec<Link>, dt: chrono::DateTime<Utc>) -> SearchResult {
    SearchResult { unique_id: n.to_string(), title: title.to_string(), content: content.to_string(), links, datetime: dt, ..Default::default() }
}

macro_rules! simple_plugin {
    ($struct:ident, $name:expr, $prio:expr) => {
        pub struct $struct;
        #[async_trait]
        impl SearchPlugin for $struct {
            fn name(&self) -> &'static str { $name }
            fn priority(&self) -> i32 { $prio }
            async fn search(&self, client: &HttpClient, keyword: &str, ext: &ExtMap) -> Result<Vec<SearchResult>> { search_impl($name, client, keyword, ext).await }
        }
    };
}

async fn search_impl(name: &str, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    match name {
        "jikepan" => jikepan(client, keyword).await,
        "meitizy" => meitizy(client, keyword).await,
        "melost" => melost(client, keyword).await,
        "yunsou" => yunsou(client, keyword).await,
        "ash" => ash(client, keyword).await,
        _ => Ok(vec![]),
    }
}

simple_plugin!(Jikepan, "jikepan", 3);
simple_plugin!(Meitizy, "meitizy", 3);
simple_plugin!(Melost, "melost", 3);
simple_plugin!(Yunsou, "yunsou", 3);
simple_plugin!(Ash, "ash", 3);

// ============ jikepan ============
async fn jikepan(client: &HttpClient, kw: &str) -> Result<Vec<SearchResult>> {
    let mut h = browser_headers(Some("https://jikepan.xyz/"));
    h.insert("content-type", HeaderValue::from_static("application/json"));
    let body = json!({"name": kw, "is_all": false}).to_string();
    let (_, resp, _) = client.request_text(reqwest::Method::POST, "https://api.jikepan.xyz/search", Some(h), Some(body)).await?;
    let v: Value = serde_json::from_str(&resp).unwrap_or(Value::Null);
    if v.get("msg").and_then(|x| x.as_str()) != Some("success") { return Ok(vec![]); }
    let mut out = Vec::new();
    if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
        for (i, item) in list.iter().enumerate() {
            let title = item.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let mut links = Vec::new();
            if let Some(ls) = item.get("links").and_then(|x| x.as_array()) {
                for l in ls {
                    let svc = l.get("service").and_then(|x| x.as_str()).unwrap_or("").to_lowercase();
                    let url = l.get("link").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                    let pwd = l.get("pwd").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                    if url.is_empty() { continue; }
                    let ty = match svc.as_str() {
                        "baidu" | "aliyun" | "xunlei" | "quark" | "115" | "123" | "pikpak" | "ed2k" | "magnet" => svc.clone(),
                        "189cloud" => "tianyi".into(),
                        "caiyun" => "mobile".into(),
                        "unknown" => continue,
                        "others" if url.contains("drive.uc.cn") => "uc".into(),
                        _ => "others".into(),
                    };
                    links.push(Link { r#type: ty, url, password: pwd, datetime: None, work_title: None });
                }
            }
            if links.is_empty() { continue; }
            out.push(r(&format!("jikepan-{}", i), &title, "", links, Utc::now()));
        }
    }
    Ok(out)
}

// ============ meitizy ============
async fn meitizy(client: &HttpClient, kw: &str) -> Result<Vec<SearchResult>> {
    let mut h = browser_headers(Some("https://video.451024.xyz/"));
    h.insert("content-type", HeaderValue::from_static("application/json"));
    let body = json!({"title": kw, "page": 1, "size": 1000}).to_string();
    let (_, resp, _) = client.request_text(reqwest::Method::POST, "https://video.451024.xyz/api/search", Some(h), Some(body)).await?;
    let v: Value = serde_json::from_str(&resp).unwrap_or(Value::Null);
    let mut out = Vec::new();
    if let Some(data) = v.get("data").and_then(|x| x.as_array()) {
        for item in data {
            let url = item.get("link").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            if url.is_empty() { continue; }
            let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let content = item.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let lt = item.get("link_type").and_then(|x| x.as_str()).unwrap_or("");
            let ty = match lt {
                "alipan" => "aliyun", "xunlei" | "baidu" | "quark" | "uc" | "115" | "123" | "tianyi" | "mobile" | "pikpak" => lt,
                _ => determine_type(&url),
            };
            let id = json_id(item.get("id"));
            let created = item.get("created_at").and_then(|x| x.as_str())
                .or_else(|| item.get("updated_at").and_then(|x| x.as_str())).unwrap_or("");
            let links = vec![Link { r#type: ty.to_string(), url, password: String::new(), datetime: None, work_title: None }];
            out.push(r(&format!("meitizy-{}", id), &title, &content, links, parse_time(created)));
        }
    }
    Ok(out)
}

// ============ melost ============
async fn melost(client: &HttpClient, kw: &str) -> Result<Vec<SearchResult>> {
    let mut pages = Vec::new();
    for page in 1..=3 {
        let mut h = browser_headers(Some("https://www.melost.cn/search"));
        h.insert("content-type", HeaderValue::from_static("application/json"));
        h.insert("origin", HeaderValue::from_static("https://www.melost.cn"));
        h.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
        let body = json!({
            "page": page, "q": kw, "user": "", "exact": false, "user_distinct": false,
            "format": [], "share_time": "", "share_year": "", "size": 30, "order": "", "type": "",
            "search_ticket": "", "exclude_user": [],
            "adv_params": {"wechat_pwd": "", "search_code": "", "platform": "pc", "fp_data": "", "automated": "0"}
        }).to_string();
        let c = client.clone();
        pages.push(async move { c.request_text(reqwest::Method::POST, "https://www.melost.cn/v1/search/disk", Some(h), Some(body)).await });
    }
    let responses = futures::future::join_all(pages).await;
    let mut out = Vec::new();
    for res in responses.into_iter().flatten() {
        let (_, resp, _) = res;
        let v: Value = serde_json::from_str(&resp).unwrap_or(Value::Null);
        if v.get("code").and_then(|x| x.as_i64()) != Some(200) { continue; }
        if let Some(list) = v.get("data").and_then(|d| d.get("list")).and_then(|x| x.as_array()) {
            for (i, item) in list.iter().enumerate() {
                let url = item.get("link").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if url.is_empty() { continue; }
                let title = strip_tags(item.get("disk_name").and_then(|x| x.as_str()).unwrap_or(""));
                let pwd = item.get("disk_pass").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                let files = item.get("files").and_then(|x| x.as_str()).unwrap_or("");
                let user = item.get("share_user").and_then(|x| x.as_str()).unwrap_or("");
                let tags: Vec<String> = item.get("tags").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
                let content = format!("{}\n分享用户: {}\n标签: {}", files, user, tags.join("、"));
                let dt = item.get("shared_time").and_then(|x| x.as_str()).unwrap_or("");
                let ty = match item.get("disk_type").and_then(|x| x.as_str()).unwrap_or("").to_uppercase().as_str() {
                    "BDY" | "BAIDU" => "baidu", "ALY" | "ALIYUN" => "aliyun", "QUARK" => "quark",
                    "TIANYI" => "tianyi", "UC" => "uc", "CAIYUN" | "MOBILE" => "mobile", "115" => "115",
                    "XUNLEI" => "xunlei", "123" | "123PAN" => "123", "PIKPAK" => "pikpak", "LANZOU" => "lanzou",
                    _ => determine_type(&url),
                };
                let id = item.get("disk_id").and_then(|x| x.as_str()).unwrap_or("");
                let uid = if id.is_empty() { format!("melost-{}-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0), i) } else { format!("melost-{}", id) };
                let links = vec![Link { r#type: ty.to_string(), url, password: pwd, datetime: None, work_title: None }];
                out.push(r(&uid, &title, &content, links, parse_time(dt)));
            }
        }
    }
    Ok(out)
}

// ============ yunsou（HTML 内嵌 JSON）============
async fn yunsou(client: &HttpClient, kw: &str) -> Result<Vec<SearchResult>> {
    let url = format!("https://yunsou.xyz/s/{}.html", crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url,
        Some(browser_headers(Some("https://yunsou.xyz/"))), None).await?;
    let json_str = extract_json_var(&body, "var jsonData");
    if json_str.is_empty() { return Ok(vec![]); }
    let arr: Value = match serde_json::from_str(&json_str) { Ok(v) => v, Err(_) => return Ok(vec![]) };
    let mut out = Vec::new();
    if let Some(items) = arr.as_array() {
        for item in items {
            let id = json_id(item.get("id"));
            let url = item.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let is_type = item.get("is_type").and_then(|x| x.as_i64()).unwrap_or(-1);
            let ty = match is_type { 0 => "quark", 1 => "aliyun", 2 => "baidu", 3 => "uc", 4 => "xunlei", _ => determine_type(&url) };
            let mut pwd = item.get("code").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if pwd.is_empty() { pwd = url_password(&url); }
            let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let cat = item.get("category").and_then(|c| c.get("name")).and_then(|x| x.as_str()).unwrap_or("");
            let content = if cat.is_empty() { name.clone() } else { format!("【{}】{}", cat, name) };
            let times = item.get("times").and_then(|x| x.as_str()).unwrap_or("");
            let links = vec![Link { r#type: ty.to_string(), url, password: pwd, datetime: None, work_title: None }];
            out.push(r(&format!("yunsou-{}", id), &title, &content, links, parse_time(times)));
        }
    }
    Ok(out)
}

/// 提取 `var jsonData = '...';` 中的 JSON
pub(crate) fn extract_json_var(body: &str, marker: &str) -> String {
    let pos = match body.find(marker) { Some(p) => p, None => return String::new() };
    let rest = &body[pos + marker.len()..];
    let q1 = match rest.find('\'') { Some(p) => p, None => return String::new() };
    let after = &rest[q1 + 1..];
    let q2 = match after.find('\'') { Some(p) => p, None => return String::new() };
    after[..q2].replace("\\/", "/")
}

// ============ ash ============
async fn ash(client: &HttpClient, kw: &str) -> Result<Vec<SearchResult>> {
    let url = format!("https://so.allsharehub.com/s/{}.html", crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url,
        Some(browser_headers(Some("https://so.allsharehub.com/"))), None).await?;
    let json_str = extract_json_var(&body, "var jsonData");
    if json_str.is_empty() { return Ok(vec![]); }
    let arr: Value = match serde_json::from_str(&json_str) { Ok(v) => v, Err(_) => return Ok(vec![]) };
    let mut out = Vec::new();
    let cats = ["短剧", "电影", "电视剧", "动漫", "综艺", "充电视频"];
    if let Some(items) = arr.as_array() {
        for item in items {
            let mut url = item.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if url.contains("pan.qualk.cn") { url = url.replace("pan.qualk.cn", "pan.quark.cn"); }
            if url.len() < 8 { continue; }
            let is_type = item.get("is_type").and_then(|x| x.as_i64()).unwrap_or(0);
            let ty = match is_type { 0 => "quark", 2 => "baidu", 3 => "uc", 4 => "xunlei", _ => "quark" };
            let pwd = item.get("code").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let id = json_id(item.get("id"));
            let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let cat_id = item.get("source_category_id").and_then(|x| x.as_i64()).unwrap_or(0);
            let times = item.get("times").and_then(|x| x.as_str()).unwrap_or("");
            let links = vec![Link { r#type: ty.to_string(), url, password: pwd, datetime: None, work_title: None }];
            let mut sr = r(&format!("ash-{}", id), &title, &name, links, parse_time(times));
            if cat_id >= 1 && cat_id <= 6 { sr.tags = vec![cats[(cat_id - 1) as usize].to_string()]; }
            out.push(sr);
        }
    }
    Ok(out)
}
