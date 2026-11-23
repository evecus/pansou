//! HTML 影视类插件：muou / duoduo / zxzj / hdmoli / quarktv
//! 解析逻辑放同步函数，避免 scraper::Html 跨 await。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{self, determine_type, extract_urls, first_text, push_link, sel};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use scraper::Html;

pub struct Muou;
pub struct Duoduo;
pub struct Zxzj;
pub struct Hdmoli;
pub struct Quarktv;

#[async_trait]
impl SearchPlugin for Muou {
    fn name(&self) -> &'static str { "muou" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { maccms(c, k, "muou", "https://666.666291.xyz", e).await }
}
#[async_trait]
impl SearchPlugin for Duoduo {
    fn name(&self) -> &'static str { "duoduo" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { maccms(c, k, "duoduo", "https://tv.yydsys.top", e).await }
}
#[async_trait]
impl SearchPlugin for Zxzj {
    fn name(&self) -> &'static str { "zxzj" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { zxzj(c, k, e).await }
}
#[async_trait]
impl SearchPlugin for Hdmoli {
    fn name(&self) -> &'static str { "hdmoli" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { hdmoli(c, k, e).await }
}
#[async_trait]
impl SearchPlugin for Quarktv {
    fn name(&self) -> &'static str { "quarktv" }
    async fn search(&self, c: &HttpClient, k: &str, e: &ExtMap) -> Result<Vec<SearchResult>> { quarktv(c, k, e).await }
}

fn links_from_detail(detail_html: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let doc = Html::parse_document(detail_html);
    for e in sel(&doc, "[data-clipboard-text]") {
        if let Some(u) = e.value().attr("data-clipboard-text") { push_link(&mut out, u.trim()); }
    }
    for a in sel(&doc, "#download-list a[href], .module-row-one a[href]") {
        if let Some(u) = a.value().attr("href") { push_link(&mut out, u.trim()); }
    }
    for u in extract_urls(detail_html) { push_link(&mut out, &u); }
    out
}

// ============ muou / duoduo ============
fn maccms_parse_search(body: &str) -> Vec<(String, String, String, String)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for item in sel(&doc, ".module-search-item") {
        let a = sel(&item, ".video-info-header h3 a").into_iter().next();
        let href = match a { Some(a) => a.value().attr("href").unwrap_or("").to_string(), None => continue };
        let title = sel(&item, ".video-info-header h3 a").into_iter().next().map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let id = regex::Regex::new(r"/vod/detail/id/(\d+)\.html").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        let quality = first_text(&item, ".video-serial");
        let tags: Vec<String> = sel(&item, ".video-info-aux .tag-link a").into_iter().map(|e| e.text().collect::<String>().trim().to_string()).filter(|s| !s.is_empty()).collect();
        let content = if quality.is_empty() { tags.join("、") } else { format!("【{}】{}", quality, tags.join("、")) };
        items.push((href, title, id, content));
    }
    items
}

async fn maccms(client: &HttpClient, kw: &str, name: &str, base: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let url = format!("{}/index.php/vod/search/wd/{}.html", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = maccms_parse_search(&body);
    let mut out = Vec::new();
    for (href, title, id, content) in items {
        let detail_url = if href.starts_with("http") { href.clone() } else { format!("{}{}", base, href) };
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
        let links = links_from_detail(&dbody);
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("{}-{}", name, id), title, content, datetime: Utc::now(), links, ..Default::default() });
    }
    Ok(out)
}

// ============ zxzj ============
fn zxzj_parse_search(body: &str, base: &str) -> Vec<(String, String, String)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for li in sel(&doc, "ul.stui-vodlist li") {
        let a = sel(&li, ".stui-vodlist__detail h4.title a").into_iter().next();
        let href = match a { Some(a) => a.value().attr("href").unwrap_or("").to_string(), None => continue };
        let title = sel(&li, ".stui-vodlist__detail h4.title a").into_iter().next().map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let id = regex::Regex::new(r"/detail/(\d+)\.html").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        items.push((abs_url(base, &href), title, id));
        if items.len() >= 10 { break; }
    }
    items
}

fn zxzj_parse_player(pbody: &str) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(c) = regex::Regex::new(r"var\s+player_aaaa\s*=\s*(\{[^;]+\})").ok().and_then(|r| r.captures(pbody)) {
        let js = c.get(1).unwrap().as_str().replace("\\/", "/");
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&js) {
            if let Some(u) = v.get("url").and_then(|x| x.as_str()) { urls.push(u.to_string()); }
        }
    }
    urls
}

fn zxzj_parse_detail(dbody: &str, base: &str) -> Vec<String> {
    let doc = Html::parse_document(dbody);
    sel(&doc, "ul.stui-content__playlist li a[href]").into_iter()
        .map(|a| abs_url(base, a.value().attr("href").unwrap_or(""))).collect()
}

async fn zxzj(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://www.zxzjhd.com";
    let url = format!("{}/vodsearch/-------------.html?wd={}&submit=", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = zxzj_parse_search(&body, base);
    let mut out = Vec::new();
    for (detail_url, title, id) in items {
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
        let play_urls = zxzj_parse_detail(&dbody, base);
        let mut links = Vec::new();
        for u in extract_urls(&dbody) { push_link(&mut links, &u); }
        for purl in play_urls {
            if let Ok((_, pbody, _)) = client.request_text(reqwest::Method::GET, &purl, Some(browser_headers(Some(base))), None).await {
                for u in zxzj_parse_player(&pbody) { push_link(&mut links, &u); }
            }
            if links.len() > 20 { break; }
        }
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("zxzj-{}", id), title, datetime: Utc::now(), links, ..Default::default() });
    }
    Ok(out)
}

fn abs_url(base: &str, href: &str) -> String {
    if href.starts_with("//") { format!("https:{}", href) }
    else if href.starts_with("http") { href.to_string() }
    else if href.starts_with('/') { format!("{}{}", base, href) }
    else { format!("{}/{}", base, href) }
}

// ============ hdmoli ============
fn hdmoli_parse_search(body: &str, base: &str) -> Vec<(String, String, String)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for li in sel(&doc, "#searchList > li.active.clearfix") {
        let a = sel(&li, ".detail h4.title a").into_iter().next();
        let (href, title) = match a { Some(a) => (a.value().attr("href").unwrap_or("").to_string(), a.text().collect::<String>().trim().to_string()), None => continue };
        let detail_url = if href.starts_with('/') { format!("{}{}", base, href) } else if href.starts_with("http") { href.clone() } else { format!("{}/{}", base, href) };
        let content = sel(&li, ".detail p").into_iter().map(|e| e.text().collect::<String>().trim().to_string()).collect::<Vec<_>>().join("\n");
        items.push((detail_url, title, content));
        if items.len() >= 50 { break; }
    }
    items
}

fn hdmoli_parse_detail(dbody: &str) -> Vec<Link> {
    let doc = Html::parse_document(dbody);
    let mut links = Vec::new();
    for p in sel(&doc, ".downlist p") {
        let ptext = p.text().collect::<String>();
        let is_quark = ptext.contains("夸 克") || ptext.contains("夸克");
        let is_baidu = ptext.contains("百 度") || ptext.contains("百度");
        if !is_quark && !is_baidu { continue; }
        for a in sel(&p, "a[href]") {
            if let Some(u) = a.value().attr("href") { push_link(&mut links, u.trim()); }
        }
    }
    if links.is_empty() {
        if let Ok(re_q) = regex::Regex::new(r#"href\s*=\s*["']([^"']*pan\.quark\.cn[^"']*)["']"#) {
            for c in re_q.captures_iter(dbody) { push_link(&mut links, c.get(1).map(|m| m.as_str()).unwrap_or("")); }
        }
        if let Ok(re_b) = regex::Regex::new(r#"href\s*=\s*["']([^"']*pan\.baidu\.com[^"']*)["']"#) {
            for c in re_b.captures_iter(dbody) { push_link(&mut links, c.get(1).map(|m| m.as_str()).unwrap_or("")); }
        }
    }
    links
}

async fn hdmoli(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://www.hdmoli.pro";
    let url = format!("{}/search.php?searchkey={}&submit=", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = hdmoli_parse_search(&body, base);
    let mut out = Vec::new();
    for (detail_url, title, content) in items {
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
        let links = hdmoli_parse_detail(&dbody);
        if links.is_empty() { continue; }
        let uid = format!("hdmoli-{}-{}", out.len() + 1, Utc::now().timestamp());
        out.push(SearchResult { unique_id: uid, title, content, datetime: Utc::now(), links, ..Default::default() });
    }
    Ok(out)
}

// ============ quarktv ============
fn quarktv_parse_search(body: &str, base: &str) -> Vec<(String, String, String, String, String, String)> {
    let doc = Html::parse_document(body);
    let mut items = Vec::new();
    for art in sel(&doc, "article.excerpt") {
        let a = sel(&art, "header h2 a").into_iter().next();
        let (href, title) = match a { Some(a) => (a.value().attr("href").unwrap_or("").to_string(), a.text().collect::<String>().trim().to_string()), None => continue };
        let id = regex::Regex::new(r"/(\d+)\.html?$").ok().and_then(|r| r.captures(&href)).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
        let summary = first_text(&art, "p.note");
        let cat = first_text(&art, "div.meta a.cat");
        let raw_time = first_text(&art, "div.meta time");
        items.push((abs_url(base, &href), title, id, summary, cat, raw_time));
    }
    items
}

fn quarktv_parse_postid(dbody: &str) -> String {
    let doc = Html::parse_document(dbody);
    sel(&doc, "div.postkit").into_iter().next().and_then(|e| e.value().attr("data-postid").map(|s| s.to_string())).unwrap_or_default()
}

fn quarktv_parse_resources(abody: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(abody) {
        if let Some(res) = v.get("resources").and_then(|x| x.as_array()) {
            for r in res {
                let u = r.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
                if u.is_empty() { continue; }
                let cls = r.get("cls").and_then(|x| x.as_str()).unwrap_or("");
                let is_bt = r.get("is_bt").and_then(|x| x.as_bool()).unwrap_or(false);
                let ty = if is_bt || u.starts_with("magnet:") { "magnet" }
                    else if cls.contains("quark") { "quark" } else if cls.contains("baidu") { "baidu" }
                    else if cls.contains("uc") { "uc" } else if cls.contains("xunlei") { "xunlei" }
                    else if cls.contains("ali") { "aliyun" } else { determine_type(u) };
                out.push((ty.to_string(), u.to_string()));
            }
        }
    }
    out
}

async fn quarktv(client: &HttpClient, kw: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
    let base = "https://www.quarktv.com";
    let url = format!("{}/?s={}", base, crate::util::text::url_encode(kw));
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(browser_headers(Some(base))), None).await?;
    let items = quarktv_parse_search(&body, base);
    let mut out = Vec::new();
    for (detail_url, title, id, summary, cat, raw_time) in items {
        let (_, dbody, _) = match client.request_text(reqwest::Method::GET, &detail_url, Some(browser_headers(Some(base))), None).await { Ok(v) => v, Err(_) => continue };
        let postid = quarktv_parse_postid(&dbody);
        if postid.is_empty() { continue; }
        let api = format!("{}/wp-json/postkit/v1/pan_view?post_id={}&visitor_id=pk_{}", base, postid, Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let mut ah = browser_headers(Some(&detail_url));
        ah.insert("accept", reqwest::header::HeaderValue::from_static("application/json"));
        let (_, abody, _) = match client.request_text(reqwest::Method::GET, &api, Some(ah), None).await { Ok(v) => v, Err(_) => continue };
        let res = quarktv_parse_resources(&abody);
        let mut links = Vec::new();
        for (ty, u) in res {
            push_link(&mut links, &u);
            if let Some(l) = links.last_mut() { l.r#type = ty; }
        }
        if links.is_empty() { continue; }
        out.push(SearchResult { unique_id: format!("quarktv-{}", id), title, content: summary, tags: if cat.is_empty() { vec![] } else { vec![cat] }, datetime: helpers::parse_time(&raw_time), links, ..Default::default() });
    }
    Ok(out)
}
