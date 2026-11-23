//! Apple CMS（hl-* / 影视类）插件（已验证可用）：fox4k
//! 解析逻辑同步化，避免 scraper Html 跨 await。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{determine_type, first_text, password_near, sel, url_password};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue};
use scraper::Html;

fn q(s: &str) -> String { crate::util::text::url_encode(s) }

fn detail_headers() -> HeaderMap {
    browser_headers(Some("https://4kfox.com/"))
}

fn random_private_ip() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    match rng.gen_range(0..3) {
        0 => format!("192.168.{}.{}", rng.gen_range(0..255), rng.gen_range(1..255)),
        1 => format!("10.{}.{}.{}", rng.gen_range(0..255), rng.gen_range(0..255), rng.gen_range(1..255)),
        _ => format!("172.{}.{}.{}", rng.gen_range(16..32), rng.gen_range(0..255), rng.gen_range(1..255)),
    }
}

fn search_headers() -> HeaderMap {
    let mut h = detail_headers();
    if let Ok(v) = HeaderValue::from_str(&random_private_ip()) {
        h.insert("x-forwarded-for", v.clone());
        h.insert("x-real-ip", v);
    }
    h
}

static QUARK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://pan\.quark\.cn/s/[0-9a-zA-Z]+").unwrap());
static MAGNET_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"magnet:\?xt=urn:btih:[0-9a-fA-F]{40}[^\s"']*"#).unwrap());
static ED2K_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"ed2k://\|file\|[^|]+\|[^|]+\|[^|]+\|/?").unwrap());
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"']+"#).unwrap());
static ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/video/(\d+)\.html").unwrap());
static PAGES_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/\s*(\d+)").unwrap());

fn fox4k_links_from_text(text: &str) -> Vec<Link> {
    let mut out = Vec::new();
    if QUARK_RE.is_match(text) { return out; } // 排除含夸克的整个文本块
    for m in MAGNET_RE.find_iter(text) {
        out.push(Link { r#type: "magnet".into(), url: m.as_str().into(), password: String::new(), datetime: None, work_title: None });
    }
    for m in ED2K_RE.find_iter(text) {
        out.push(Link { r#type: "ed2k".into(), url: m.as_str().into(), password: String::new(), datetime: None, work_title: None });
    }
    for m in URL_RE.find_iter(text) {
        let url = m.as_str().trim_end_matches(|c: char| matches!(c, ',' | '。' | '，' | ';'));
        if QUARK_RE.is_match(url) { continue; }
        let ty = determine_type(url);
        if ty == "others" { continue; }
        if out.iter().any(|l: &Link| l.url == url) { continue; }
        let pass = { let p = url_password(url); if p.is_empty() { password_near(text) } else { p } };
        out.push(Link { r#type: ty.into(), url: url.into(), password: pass, datetime: None, work_title: None });
    }
    out
}

fn fox4k_parse_list(body: &str, keyword: &str) -> (Vec<(String, String)>, usize) {
    // 返回 [(id, detail_url)] 和总页数
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for el in sel(&doc, ".hl-list-item") {
        let a = match sel(&el, ".hl-item-pic a").into_iter().next() { Some(e) => e, None => continue };
        let href = a.value().attr("href").unwrap_or("").to_string();
        if href.is_empty() { continue; }
        let href = if href.starts_with('/') { format!("https://4kfox.com{}", href) } else { href };
        let id = match ID_RE.captures(&href).and_then(|c| c.get(1)) { Some(m) => m.as_str().to_string(), None => continue };
        let title = first_text(&el, ".hl-item-title a");
        if title.is_empty() || !crate::util::text::matches_keywords(&title, "", keyword) { continue; }
        items.push((id, href));
    }
    let pages = first_text(&doc, ".hl-page-tips a").split('/').nth(1)
        .and_then(|s| PAGES_RE.captures(s)).and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<usize>().ok()).unwrap_or(1);
    (items, pages.min(10))
}

fn fox4k_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    // 全文先扫一遍
    let full = doc.root_element().text().collect::<String>();
    for l in fox4k_links_from_text(&full) {
        if !out.iter().any(|x: &Link| x.url == l.url) { out.push(l); }
    }
    // 下载区 + 播放区
    for sec in sel(&doc, ".hl-rb-downlist, .hl-rb-playlist") {
        let txt = sec.text().collect::<String>();
        for l in fox4k_links_from_text(&txt) {
            if !out.iter().any(|x: &Link| x.url == l.url) { out.push(l); }
        }
        for a in sel(&sec, "a[href], .down-copy[data-clipboard-text]") {
            let href = a.value().attr("data-clipboard-text").or_else(|| a.value().attr("href")).unwrap_or("");
            if href.is_empty() { continue; }
            for l in fox4k_links_from_text(href) {
                if !out.iter().any(|x: &Link| x.url == l.url) { out.push(l); }
            }
        }
    }
    out
}

pub struct Fox4k;

#[async_trait]
impl SearchPlugin for Fox4k {
    fn name(&self) -> &'static str { "fox4k" }
    fn priority(&self) -> i32 { 3 }

    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let first = format!("https://4kfox.com/search/{}-------------.html", q(keyword));
        let body = client.get_text(&first, Some(search_headers())).await?;
        let (items, pages) = fox4k_parse_list(&body, keyword);
        let mut all_items = items;
        // 抓额外页
        if pages > 1 {
            let mut page_futs = Vec::new();
            for p in 2..=pages {
                let url = format!("https://4kfox.com/search/{}----------{}---.html", q(keyword), p);
                let c = client.clone();
                page_futs.push(async move { c.get_text(&url, Some(search_headers())).await });
            }
            for b in futures::future::join_all(page_futs).await.into_iter().flatten() {
                let (it, _) = fox4k_parse_list(&b, keyword);
                all_items.extend(it);
            }
        }
        // 去重 id
        let mut seen = std::collections::HashSet::new();
        all_items.retain(|(id, _)| seen.insert(id.clone()));
        if all_items.is_empty() { return Ok(Vec::new()); }

        // 并发抓详情
        let mut futs = Vec::new();
        for (id, url) in all_items {
            let c = client.clone();
            futs.push(async move {
                let dbody = c.get_text(&url, Some(detail_headers())).await.ok();
                (id, url, dbody)
            });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (id, _detail_url, dbody) in details {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = fox4k_parse_detail(&dbody);
            if links.is_empty() { continue; }
            let mut r = SearchResult {
                unique_id: format!("fox4k-{}", id),
                datetime: Utc::now(),
                channel: String::new(),
                ..Default::default()
            };
            // 详情补充标题/简介
            let doc = Html::parse_document(&dbody);
            r.title = first_text(&doc, "h2.hl-dc-title");
            r.content = first_text(&doc, ".hl-content-wrap .hl-content-text");
            for li in sel(&doc, ".hl-vod-data ul li") {
                let t = li.text().collect::<String>().replace('：', ": ");
                if t.contains("类型:") || t.contains("地区:") || t.contains("语言:") { r.tags.push(t.trim().to_string()); }
            }
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}
