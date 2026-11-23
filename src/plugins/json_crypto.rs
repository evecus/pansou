//! JSON/API 类插件（解密组）：sdso（AES）/ yunso（XOR + HTML片段）

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::*;
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::header::HeaderValue;
use scraper::{Html, Selector};
use serde_json::Value;

pub struct Sdso;
pub struct Yunso;

#[async_trait]
impl SearchPlugin for Sdso {
    fn name(&self) -> &'static str { "sdso" }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> { sdso(client, keyword).await }
}

#[async_trait]
impl SearchPlugin for Yunso {
    fn name(&self) -> &'static str { "yunso" }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> { yunso(client, keyword).await }
}

// ============ sdso ============
async fn sdso(client: &HttpClient, kw: &str) -> Result<Vec<SearchResult>> {
    let key = b"4OToScUFOaeVTrHE";
    let iv = b"9CLGao1vHKqm17Oz";
    let mut tasks = Vec::new();
    for from in ["baidu", "quark", "xunlei", "ali"] {
        for page in 1..=2 {
            let url = format!("https://sdso.top/api/sd/search?name={}&pageNo={}&from={}",
                crate::util::text::url_encode(kw), page, from);
            let mut h = browser_headers(Some("https://sdso.top/"));
            h.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
            let c = client.clone();
            let f = from.to_string();
            tasks.push(async move {
                let r = c.request_text(reqwest::Method::GET, &url, Some(h), None).await;
                (r, f, page)
            });
        }
    }
    let responses = futures::future::join_all(tasks).await;
    let mut out = Vec::new();
    for (res, from, page) in responses {
        let (_, body, _) = match res { Ok(v) => v, Err(_) => continue };
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if v.get("code").and_then(|x| x.as_i64()) != Some(200) { continue; }
        let list = match v.get("data").and_then(|d| d.get("list")).and_then(|x| x.as_array()) { Some(l) => l, None => continue };
        for item in list {
            let enc = item.get("url").and_then(|x| x.as_str()).unwrap_or("");
            let decrypted = match aes_cbc_decrypt_b64(enc, key, iv) { Ok(d) => d, Err(_) => continue };
            if !(decrypted.contains("pan.quark.cn") || decrypted.contains("pan.xunlei.com") ||
                 decrypted.contains("aliyundrive.com") || decrypted.contains("alipan.com") || decrypted.contains("pan.baidu.com")) { continue; }
            let ty = match from.as_str() { "quark" => "quark", "xunlei" => "xunlei", "ali" | "aliyun" => "aliyun", "baidu" => "baidu", _ => "others" };
            let title = strip_tags(item.get("name").and_then(|x| x.as_str()).unwrap_or(""));
            let creator = item.get("creatorName").and_then(|x| x.as_str()).unwrap_or("");
            let fcount = item.get("fileCount").and_then(|x| x.as_i64()).unwrap_or(0);
            let content = format!("分享者: {} | 文件数量: {} | 网盘类型: {}", creator, fcount, from);
            let id = json_id(item.get("id"));
            let share_time = item.get("gmtShare").and_then(|x| x.as_str()).unwrap_or("");
            let links = vec![Link { r#type: ty.to_string(), url: decrypted, password: String::new(), datetime: None, work_title: None }];
            out.push(SearchResult {
                unique_id: format!("sdso-{}-{}-{}", id, from, page), title, content, links,
                tags: vec![from.clone(), ty.to_string()], datetime: parse_time(share_time), ..Default::default()
            });
        }
    }
    Ok(out)
}

// ============ yunso ============
async fn yunso(client: &HttpClient, kw: &str) -> Result<Vec<SearchResult>> {
    let key = "pWz1vnL1fTkOvTMW3f9M1jJWfneUIh50";
    let kw_enc = crate::util::text::url_encode(kw);
    let mut tasks = Vec::new();
    for page in 1..=3 {
        let url = format!("https://www.yunso.net/api/Core/search2?requestID=&mode=90002&scope_content=0&stype=&wd={}&uk=&page={}&limit=20&screen_filetype=", kw_enc, page);
        let mut h = browser_headers(Some(&format!("https://www.yunso.net/index/user/s?wd={}", kw_enc)));
        h.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
        h.insert("origin", HeaderValue::from_static("https://www.yunso.net"));
        h.insert("x-requested-with", HeaderValue::from_static("XMLHttpRequest"));
        let c = client.clone();
        tasks.push(async move { c.request_text(reqwest::Method::POST, &url, Some(h), None).await });
    }
    let responses = futures::future::join_all(tasks).await;
    let mut out = Vec::new();
    for res in responses.into_iter().flatten() {
        let (_, body, _) = res;
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if v.get("code").and_then(|x| x.as_i64()) != Some(0) { continue; }
        let html = match v.get("data").and_then(|x| x.as_str()) { Some(d) => d, None => continue };
        let wrapped = format!("<div id=\"yunso-root\">{}</div>", html);
        let doc = Html::parse_document(&wrapped);
        let card_sel = Selector::parse("div.layui-card[data-qid]").unwrap();
        let a_sel = Selector::parse("a[onclick*=\"open_sid\"]").unwrap();
        for card in doc.select(&card_sel) {
            let qid = card.value().attr("data-qid").unwrap_or("").to_string();
            let a = match card.select(&a_sel).next() { Some(a) => a, None => continue };
            let title = a.text().collect::<String>().trim().to_string();
            let enc_url = a.value().attr("url").unwrap_or("");
            let pa = a.value().attr("pa").unwrap_or("");
            let full_id = a.value().attr("id").unwrap_or("");
            let url = xor_decrypt_b64(enc_url, key);
            if url.is_empty() { continue; }
            let mut pwd = url_password(&url);
            if pwd.is_empty() && !pa.is_empty() { pwd = xor_decrypt_b64(pa, key); }
            let typecode = card.select(&Selector::parse("img[src*=\"/assets/xyso/\"]").unwrap()).next()
                .and_then(|img| img.value().attr("src"))
                .and_then(|src| {
                    let re = regex::Regex::new(r"/assets/xyso/(\d+)\.png").unwrap();
                    re.captures(src).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
                }).unwrap_or_default();
            let ty = match typecode.as_str() {
                "1" => "baidu", "20100" => "aliyun", "20500" => "quark", "20000" => "tianyi",
                "20300" => "mobile", "20400" => "xunlei", "20501" => "uc", "20600" => "lanzou",
                _ => determine_type(&url),
            };
            let datetime = card.select(&Selector::parse(".layui-card-header").unwrap()).next()
                .map(|e| e.text().collect::<String>())
                .and_then(|t| {
                    let re = regex::Regex::new(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}").unwrap();
                    re.find(&t).map(|m| m.as_str().to_string())
                }).unwrap_or_default();
            let uid = if !full_id.is_empty() { format!("yunso-{}", full_id) } else { format!("yunso-{}", qid) };
            let links = vec![Link { r#type: ty.to_string(), url, password: pwd, datetime: None, work_title: None }];
            out.push(SearchResult { unique_id: uid, title, datetime: parse_time(&datetime), links, ..Default::default() });
        }
    }
    Ok(out)
}
