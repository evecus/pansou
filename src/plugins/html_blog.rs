//! HTML 两步类插件（博客/CMS）：kkv / alupan / xiaoji
//! 解析逻辑放在同步函数中，避免 scraper::Html 进入 async 状态机（非 Send）。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{self, determine_type, extract_urls, first_text, password_near, sel, url_password};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use scraper::Html;

pub struct Kkv;
pub struct Alupan;
pub struct Xiaoji;

#[async_trait]
impl SearchPlugin for Kkv {
    fn name(&self) -> &'static str { "kkv" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { kkv(c, k, e).await }
}
#[async_trait]
impl SearchPlugin for Alupan {
    fn name(&self) -> &'static str { "alupan" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { alupan(c, k, e).await }
}
#[async_trait]
impl SearchPlugin for Xiaoji {
    fn name(&self) -> &'static str { "xiaoji" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { xiaoji(c, k, e).await }
}

// ============ kkv ============
fn kkv_parse_search(body: &str, kw: &str) -> Vec<(String, String, String)> {
    let doc = Html::parse_document(body);
    let lower_kw = kw.to_lowercase();
    let mut items = Vec::new();
    for post in sel(&doc, "article.post") {
        let a = sel(&post, ".entry-header h2.entry-title a").into_iter().next();
        let (href, title) = match a {
            Some(a) => (a.value().attr("href").unwrap_or("").to_string(), a.text().collect::<String>().trim().to_string()),
            None => continue,
        };
        if !title.to_lowercase().contains(&lower_kw) { continue; }
        let id = regex::Regex::new(r"\?p=(\d+)").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        items.push((href, title, id));
        if items.len() >= 10 { break; }
    }
    items
}

fn kkv_parse_detail(dbody: &str) -> (String, String, Vec<Link>) {
    let ddoc = Html::parse_document(dbody);
    let desc = sel(&ddoc, ".entry-content p").into_iter().next()
        .map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
    let desc = if desc.len() > 200 { format!("{}...", &desc[..200]) } else { desc };
    let dt = sel(&ddoc, "time.updated").into_iter().next()
        .and_then(|e| e.value().attr("datetime").map(|s| s.to_string())).unwrap_or_default();
    let mut links = Vec::new();
    for a in sel(&ddoc, ".entry-content p a[href]") {
        let u = a.value().attr("href").unwrap_or("").trim().to_string();
        if u.is_empty() { continue; }
        let ty = determine_type(&u);
        if ty == "others" { continue; }
        let mut pwd = url_password(&u);
        if pwd.is_empty() {
            let end = dbody.find(&u).unwrap_or(0).saturating_add(u.len()).min(dbody.len());
            pwd = password_near(&dbody[..end]);
        }
        helpers::push_link(&mut links, &u);
        if let Some(l) = links.last_mut() { l.password = pwd; }
    }
    (desc, dt, links)
}

async fn kkv(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "http://kkv.q-23.cn";
    let url = format!("{}/?s={}", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = kkv_parse_search(&body, kw);
    let mut out = Vec::new();
    for (href, title, id) in items {
        let detail_url = if href.starts_with("http") { href.clone() } else { format!("{}{}", base, href) };
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
        let (desc, dt, links) = kkv_parse_detail(&dbody);
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("kkv-{}", id), title, content: desc, datetime: helpers::parse_time(&dt), links, ..Default::default() });
    }
    Ok(out)
}

// ============ alupan ============
fn alupan_parse_search(body: &str) -> Vec<(String, String, String, String, String)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for post in sel(&doc, "article.excerpt") {
        let a = sel(&post, "header h2 a").into_iter().next();
        let (href, title) = match a { Some(a) => (a.value().attr("href").unwrap_or("").to_string(), a.text().collect::<String>().trim().to_string()), None => continue };
        let id = regex::Regex::new(r"\?p=(\d+)").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        let tag = first_text(&post, "header .label");
        let summary = first_text(&post, "p.note");
        items.push((href, title, id, tag, summary));
    }
    items
}

fn alupan_parse_detail(dbody: &str) -> Vec<Link> {
    let ddoc = Html::parse_document(dbody);
    let mut links = Vec::new();
    for a in sel(&ddoc, ".article-content a[href]") {
        let u = a.value().attr("href").unwrap_or("").trim().to_string();
        if !(u.contains("pan.quark.cn") || u.contains("aliyundrive.com") || u.contains("alipan.com")) { continue; }
        helpers::push_link(&mut links, &u);
        if let Some(l) = links.last_mut() {
            if url_password(&u).is_empty() {
                let end = dbody.find(&u).unwrap_or(0).saturating_add(u.len()).min(dbody.len());
                l.password = password_near(&dbody[..end]);
            }
        }
    }
    links
}

async fn alupan(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://www.aliupan.com";
    let url = format!("{}/?s={}", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = alupan_parse_search(&body);
    let mut out = Vec::new();
    for (href, title, id, tag, summary) in items {
        let detail_url = if href.starts_with("http") { href.clone() } else { format!("{}{}", base, href) };
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(&detail_url))), None).await { Ok(v) => v, Err(_) => continue };
        let links = alupan_parse_detail(&dbody);
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("alupan-{}", id), title, content: summary, tags: if tag.is_empty() { vec![] } else { vec![tag] }, datetime: Utc::now(), links, ..Default::default() });
    }
    Ok(out)
}

// ============ xiaoji ============
fn xiaoji_parse_search(body: &str) -> Vec<(String, String, String, String)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for post in sel(&doc, "article.poster-item") {
        let a = sel(&post, ".poster-link").into_iter().next();
        let href = match a { Some(a) => a.value().attr("href").unwrap_or("").to_string(), None => continue };
        let title = first_text(&post, ".poster-title a");
        let id = regex::Regex::new(r"/(\d+)\.html").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        let cat = first_text(&post, ".poster-category a");
        let rating = first_text(&post, ".rating-score");
        items.push((href, title, id, format!("分类: {} | 评分: {}", cat, rating)));
    }
    items
}

fn xiaoji_parse_detail(dbody: &str) -> Vec<Link> {
    let ddoc = Html::parse_document(dbody);
    use base64::Engine;
    let mut links = Vec::new();
    for a in sel(&ddoc, ".resource-compact-link a[href]") {
        let raw = a.value().attr("href").unwrap_or("").trim().to_string();
        let mut real = raw.clone();
        if raw.contains("/go.html?url=") {
            if let Some(c) = regex::Regex::new(r"/go\.html\?url=([A-Za-z0-9+/]+=*)").ok().and_then(|r| r.captures(&raw)) {
                let mut b64 = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
                if let Ok(dec) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                    real = String::from_utf8_lossy(&dec).to_string();
                } else {
                    b64 = b64.replace(' ', "+");
                    while b64.len() % 4 != 0 { b64.push('='); }
                    if let Ok(dec) = base64::engine::general_purpose::STANDARD.decode(&b64) { real = String::from_utf8_lossy(&dec).to_string(); }
                }
            }
        }
        if !(real.starts_with("http") || real.starts_with("magnet:") || real.starts_with("ed2k://")) { continue; }
        helpers::push_link(&mut links, &real);
    }
    let _ = extract_urls; // 保留 import
    links
}

async fn xiaoji(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://www.xiaojitv.com";
    let url = format!("{}/?s={}", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = xiaoji_parse_search(&body);
    let mut out = Vec::new();
    for (href, title, id, content) in items {
        let detail_url = if href.starts_with("http") { href.clone() } else { format!("{}{}", base, href) };
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
        let links = xiaoji_parse_detail(&dbody);
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("xiaoji-{}", id), title, content, datetime: Utc::now(), links, ..Default::default() });
    }
    Ok(out)
}
