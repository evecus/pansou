//! HTML 复杂两步类插件：mikuclub / gaoqing888 / panta / susu
//! 所有 scraper 解析放在同步函数中，避免 Html 跨 await。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{determine_type, extract_urls, first_text, json_id, parse_time, push_link, sel, url_password};
use crate::util::http::{browser_headers, random_ua};
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::header::HeaderValue;
use scraper::Html;
use serde_json::Value;

pub struct Mikuclub;
pub struct Gaoqing888;
pub struct Panta;
pub struct Susu;

#[async_trait]
impl SearchPlugin for Mikuclub {
    fn name(&self) -> &'static str { "mikuclub" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { mikuclub(c, k, e).await }
}
#[async_trait]
impl SearchPlugin for Gaoqing888 {
    fn name(&self) -> &'static str { "gaoqing888" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { gaoqing(c, k, e).await }
}
#[async_trait]
impl SearchPlugin for Panta {
    fn name(&self) -> &'static str { "panta" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { panta(c, k, e).await }
}
#[async_trait]
impl SearchPlugin for Susu {
    fn name(&self) -> &'static str { "susu" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { susu(c, k, e).await }
}

fn push_links(out: &mut Vec<Link>, html: &str) {
    for u in extract_urls(html) {
        let ty = determine_type(&u);
        if ty == "others" && !u.starts_with("magnet:") && !u.starts_with("ed2k:") { continue; }
        if out.iter().any(|l| l.url == u) { continue; }
        let pwd = url_password(&u);
        out.push(Link { r#type: ty.to_string(), url: u, password: pwd, datetime: None, work_title: None });
    }
}

// ============ mikuclub ============
fn mikuclub_parse_detail(dbody: &str) -> Vec<Link> {
    let ddoc = Html::parse_document(dbody);
    let mut links = Vec::new();
    let container = sel(&ddoc, ".article_content").into_iter().next()
        .or_else(|| sel(&ddoc, "article.post").into_iter().next())
        .or_else(|| sel(&ddoc, ".entry-content").into_iter().next());
    if let Some(c) = container { push_links(&mut links, &c.html()); }
    else { push_links(&mut links, dbody); }
    links
}

async fn mikuclub(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let kw_enc = crate::util::text::url_encode(kw);
    let mut tasks = Vec::new();
    for cat in ["9305", "942"] {
        let url = format!("https://www.mikuclub.uk/wp-json/utils/v2/post_list?search={}&s={}&page=&pagename=search_page&page_type=search&paged=1&custom_orderby=relevance&no_cache=1&custom_cat={}", kw_enc, kw_enc, cat);
        let mut h = browser_headers(Some("https://www.mikuclub.uk/"));
        h.insert("accept", HeaderValue::from_static("application/json"));
        let c = client.clone();
        tasks.push(async move { c.request_text(reqwest::Method::GET, &url, Some(h), None).await });
    }
    let responses = futures::future::join_all(tasks).await;
    let mut posts: Vec<(String, String, String, String, String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for res in responses.into_iter().flatten() {
        let (_, body, _) = res;
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if let Some(arr) = v.get("posts").and_then(|x| x.as_array()) {
            for p in arr {
                let id = json_id(p.get("id"));
                if !seen.insert(id.clone()) { continue; }
                let href = p.get("post_href").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let title = p.get("post_title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let rank = p.get("post_rank_description").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let views = p.get("post_views").and_then(|x| x.as_i64().map(|n| n.to_string())).unwrap_or_default();
                let date = p.get("post_date").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let cat = format!("{} {}", p.get("post_main_cat_name").and_then(|x| x.as_str()).unwrap_or(""), p.get("post_cat_name").and_then(|x| x.as_str()).unwrap_or(""));
                posts.push((href, title, rank, views, date, cat.trim().to_string()));
            }
        }
    }
    let mut out = Vec::new();
    for (href, title, rank, views, date, cat) in posts {
        if href.is_empty() { continue; }
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &href, Some(browser_headers(Some(&href))), None).await { Ok(v) => v, Err(_) => continue };
        let links = mikuclub_parse_detail(&dbody);
        if links.is_empty() { continue; }
        let content = format!("口碑：{} 浏览：{}", rank, views);
        let tags = if cat.is_empty() { vec![] } else { vec![cat] };
        out.push(SearchResult { unique_id: format!("mikuclub-{}", title), title, content, tags, datetime: parse_time(&date), links, ..Default::default() });
    }
    Ok(out)
}

// ============ gaoqing888 ============
fn gaoqing_parse_search(body: &str, base: &str) -> Vec<(String, String, String, String)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for row in sel(&doc, "div.wp-list.search-list div.video-row") {
        let a = sel(&row, "a.title-link").into_iter().next();
        let (href, title) = match a { Some(a) => (a.value().attr("href").unwrap_or("").to_string(), a.text().collect::<String>().trim().to_string()), None => continue };
        let id = regex::Regex::new(r"/(\d+)/detail").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        let meta: Vec<String> = sel(&row, "div.meta").into_iter().take(2).map(|e| e.text().collect::<String>().trim().to_string()).collect();
        let detail_url = if href.starts_with("http") { href.clone() } else { format!("{}{}", base, href) };
        items.push((detail_url, title, id, meta.join(" | ")));
    }
    items
}

fn gaoqing_parse_detail(dbody: &str, base: &str) -> Vec<Link> {
    let doc = Html::parse_document(dbody);
    let mut links = Vec::new();
    for a in sel(&doc, "ul.playlist a[href], div.wp-download a[href]") {
        if let Some(h) = a.value().attr("href") { push_link_from_href(&mut links, h, base); }
    }
    if let Ok(c) = regex::Regex::new(r#"magnet:\?xt=urn:btih:[0-9A-Za-z]+[^"' <]*"#) {
        for m in c.find_iter(dbody) { push_link(&mut links, m.as_str()); }
    }
    links
}

fn push_link_from_href(out: &mut Vec<Link>, href: &str, base: &str) {
    let mut u = href.trim().to_string();
    if u.contains("/go/play?") {
        if let Ok(parsed) = url::Url::parse(&u) {
            if let Some(real) = parsed.query_pairs().find(|(k, _)| k == "url").map(|(_, v)| v.to_string()) { u = real; }
        }
    }
    if u.starts_with('/') { u = format!("{}{}", base, u); }
    push_link(out, &u);
}

async fn gaoqing(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://www.gaoqing888.com";
    let url = format!("{}/search?kw={}", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = gaoqing_parse_search(&body, base);
    let mut out = Vec::new();
    for (detail_url, title, id, content) in items {
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
        let links = gaoqing_parse_detail(&dbody, base);
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("gaoqing888-{}", id), title, content, datetime: Utc::now(), links, ..Default::default() });
    }
    Ok(out)
}

// ============ panta ============
fn panta_parse_search(body: &str, base: &str) -> Vec<(String, String, String, String, Vec<String>)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for item in sel(&doc, "div.topicItem") {
        let a = sel(&item, "a[href^='thread?topicId=']").into_iter().next();
        let href = match a { Some(a) => a.value().attr("href").unwrap_or("").to_string(), None => continue };
        let title = sel(&item, "a[href^='thread?topicId=']").into_iter().next().map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let id = regex::Regex::new(r"topicId=(\d+)").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        let summary = first_text(&item, "h2.summary");
        let item_links: Vec<String> = sel(&item, "a[href^='http']").into_iter().filter_map(|a| a.value().attr("href").map(|s| s.to_string())).collect();
        let detail_url = format!("{}/{}", base, href);
        items.push((detail_url, title, id, summary, item_links));
    }
    items
}

fn panta_parse_detail(dbody: &str) -> Vec<String> {
    let doc = Html::parse_document(dbody);
    sel(&doc, "div.topicContent a[href^='http']").into_iter()
        .filter_map(|a| a.value().attr("href").map(|s| s.to_string())).collect()
}

async fn panta(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://www.91panta.cn";
    let url = format!("{}/search?keyword={}", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = panta_parse_search(&body, base);
    let mut out = Vec::new();
    for (detail_url, title, id, summary, item_links) in items {
        let mut links = Vec::new();
        for u in &item_links { push_link(&mut links, u); }
        if links.is_empty() {
            let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
            for u in panta_parse_detail(&dbody) { push_link(&mut links, &u); }
            if links.is_empty() { push_links(&mut links, &dbody); }
        }
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("panta-{}", id), title, content: summary, datetime: Utc::now(), links, ..Default::default() });
    }
    Ok(out)
}

// ============ susu ============
fn susu_parse_search(body: &str, kw: &str) -> Vec<(String, String, String, String)> {
    let doc = Html::parse_document(body);
    let lower_kw = kw.to_lowercase();
    let mut posts = Vec::new();
    for item in sel(&doc, ".post-list-item") {
        let a = sel(&item, ".post-info h2 a").into_iter().next();
        let href = match a { Some(a) => a.value().attr("href").unwrap_or("").to_string(), None => continue };
        let title = sel(&item, ".post-info h2 a").into_iter().next().map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        if !title.to_lowercase().contains(&lower_kw) { continue; }
        let post_id = item.value().attr("id").and_then(|s| s.strip_prefix("item-")).map(|s| s.to_string())
            .or_else(|| regex::Regex::new(r"/(\d+)\.html").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()))
            .unwrap_or_default();
        let excerpt = first_text(&item, ".post-excerpt");
        let date = sel(&item, "time.b2timeago").into_iter().next().and_then(|e| e.value().attr("datetime").map(|s| s.to_string())).unwrap_or_default();
        posts.push((post_id, title, excerpt, date));
    }
    posts
}

fn susu_decode_jwt(token: &str, name: &str) -> Option<(String, String)> {
    use base64::Engine;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 { return None; }
    let mut b64 = parts[1].replace('-', "+").replace('_', "/");
    while b64.len() % 4 != 0 { b64.push('='); }
    let dec = base64::engine::general_purpose::STANDARD.decode(&b64).ok()?;
    let pv: Value = serde_json::from_slice(&dec).ok()?;
    let u = pv.get("data")?.get("url")?.as_str()?;
    let ty = determine_type(&format!("{} {}", u, name));
    if ty == "others" { return None; }
    Some((ty.to_string(), u.to_string()))
}

async fn susu(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://susuifa.com";
    let url = format!("{}/?type=post&s={}", base, crate::util::text::url_encode(kw));
    let mut h = browser_headers(Some(base));
    if let Ok(ua) = HeaderValue::from_str(random_ua()) { h.insert("user-agent", ua); }
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(h), None).await?;
    let posts = susu_parse_search(&body, kw);
    let mut out = Vec::new();
    for (post_id, title, excerpt, date) in posts {
        if post_id.is_empty() { continue; }
        let mut links = Vec::new();
        for index in 0..6 {
            let api = format!("{}/wp-json/b2/v1/getDownloadPageData?post_id={}&index=0&i={}&guest=", base, post_id, index);
            let mut ah = browser_headers(Some(&format!("{}/download?post_id={}&index=0&i={}", base, post_id, index)));
            ah.insert("content-type", HeaderValue::from_static("application/json"));
            let (_, abody, _) = match client.request_text(reqwest::Method::POST, &api, Some(ah), None).await { Ok(v) => v, Err(_) => continue };
            let v: Value = match serde_json::from_str(&abody) { Ok(v) => v, Err(_) => continue };
            let token = v.get("button").and_then(|b| b.get("url")).and_then(|x| x.as_str()).unwrap_or("");
            let name = v.get("button").and_then(|b| b.get("name")).and_then(|x| x.as_str()).unwrap_or("");
            if token.is_empty() { continue; }
            if let Some((ty, u)) = susu_decode_jwt(token, name) {
                if !links.iter().any(|l: &Link| l.url == u) {
                    links.push(Link { r#type: ty, url: u, password: String::new(), datetime: None, work_title: None });
                }
            }
        }
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("susu-{}", post_id), title, content: excerpt, datetime: parse_time(&date), links, ..Default::default() });
    }
    Ok(out)
}
