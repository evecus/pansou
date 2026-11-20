//! 两步 HTML 插件（测速 OK）：duanjuw / huban / shandian / jutoushe / xinjuc / u3c3
//! 关键：所有 HTML 解析抽成同步函数返回纯数据，绝不让 scraper::Html 跨 await。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{determine_type, first_text, password_near, sel, url_password};
use crate::plugins::html_ok2::{context_around_generic, extract_links_generic};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use md5::Digest;
use once_cell::sync::Lazy;
use regex::Regex;
use scraper::Html;

fn q(s: &str) -> String { crate::util::text::url_encode(s) }
fn result(name: &str, uid: &str, title: &str) -> SearchResult {
    SearchResult { unique_id: format!("{}-{}", name, uid), title: title.to_string(), datetime: Utc::now(), channel: String::new(), ..Default::default() }
}
fn detail_links(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let full = doc.root_element().text().collect::<String>();
    let mut links = extract_links_generic(&doc, &full);
    for l in &mut links {
        if l.password.is_empty() { l.password = password_near(&context_around_generic(&full, &l.url)); }
    }
    links
}

// ============================================================
// duanjuw：https://sm3.cc/search.php?q=&page=1（两步）
// ============================================================
pub struct Duanjuw;
static DJW_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[?&]id=(\d+)").unwrap());
static DJW_URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"']+"#).unwrap());

fn djw_normalize(href: &str) -> String {
    if href.starts_with("//") { format!("https:{}", href) }
    else if href.starts_with("http") { href.to_string() }
    else if href.starts_with('/') { format!("https://sm3.cc{}", href) }
    else { format!("https://sm3.cc/{}", href) }
}
fn djw_parse_list(body: &str) -> Vec<(String, String, String)> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for li in sel(&doc, "li.col-6") {
        let a = match sel(&li, "h3.f-14 a").into_iter().next() { Some(e) => e, None => continue };
        let title = a.text().collect::<String>().trim().to_string();
        let href = a.value().attr("href").unwrap_or("");
        if title.is_empty() || href.is_empty() { continue; }
        out.push((djw_normalize(href), title, a.value().attr("title").unwrap_or("").to_string()));
    }
    out
}
fn djw_detail_links(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let content = sel(&doc, "div.content").into_iter().next()
        .or_else(|| sel(&doc, "div.tx-text").into_iter().next());
    let full = doc.root_element().text().collect::<String>();
    let txt = content.map(|c| c.text().collect::<String>()).unwrap_or_else(|| full.clone());
    let mut out = Vec::new();
    for m in DJW_URL_RE.find_iter(&txt) {
        let url = m.as_str();
        let ty = determine_type(url);
        if ty == "others" { continue; }
        if out.iter().any(|l: &Link| l.url == url) { continue; }
        let pass = { let p = url_password(url); if p.is_empty() { password_near(&context_around_generic(&txt, url)) } else { p } };
        out.push(Link { r#type: ty.to_string(), url: url.to_string(), password: pass, datetime: None, work_title: None });
    }
    if out.is_empty() { out = detail_links(body); }
    out
}

#[async_trait]
impl SearchPlugin for Duanjuw {
    fn name(&self) -> &'static str { "duanjuw" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let mut headers = browser_headers(Some("https://sm3.cc/"));
        headers.insert("user-agent", reqwest::header::HeaderValue::from_static("Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148"));
        let body = client.get_text(&format!("https://sm3.cc/search.php?q={}&page=1", q(keyword)), Some(headers.clone())).await?;
        let items = djw_parse_list(&body); // 同步解析，doc 不跨 await
        let mut futs = Vec::new();
        for (detail, title, content) in items {
            let c = client.clone();
            let h = headers.clone();
            futs.push(async move { let b = c.get_text(&detail, Some(h)).await.ok(); (detail, title, content, b) });
        }
        let mut out = Vec::new();
        for (detail, title, content, dbody) in futures::future::join_all(futs).await {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = djw_detail_links(&dbody);
            if links.is_empty() { continue; }
            let id = DJW_ID_RE.captures(&detail).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_else(|| q(&detail));
            let mut r = result("duanjuw", &id, &title);
            r.content = content; r.links = links; out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// huban / shandian：macCMS 两步（.module-search-item → #download-list）
// ============================================================
fn maccms_parse_list(body: &str, id_re: &Regex) -> Vec<(String, String)> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for el in sel(&doc, ".module-search-item") {
        let a = match sel(&el, ".video-info-header h3 a").into_iter().next() { Some(e) => e, None => continue };
        let title = a.text().collect::<String>().trim().to_string();
        let href = a.value().attr("href").unwrap_or("");
        if let Some(id) = id_re.captures(href).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()) {
            out.push((id, title));
        }
    }
    out
}
fn maccms_detail_links(body: &str, only_uc: bool) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let mut links = Vec::new();
    for row in sel(&doc, "#download-list .module-row-one") {
        let mut urls = Vec::new();
        for e in sel(&row, "[data-clipboard-text]") {
            if let Some(v) = e.value().attr("data-clipboard-text") { urls.push(v.to_string()); }
        }
        for a in sel(&row, "a[href]") {
            if let Some(v) = a.value().attr("href") { urls.push(v.to_string()); }
        }
        for u in urls {
            if only_uc && !u.contains("drive.uc.cn") { continue; }
            let ty = if only_uc { "uc" } else { determine_type(&u) };
            if ty == "others" && !only_uc { continue; }
            if links.iter().any(|l: &Link| l.url == u) { continue; }
            let pass = if only_uc { String::new() } else { url_password(&u) };
            links.push(Link { r#type: ty.to_string(), url: u, password: pass, datetime: None, work_title: None });
        }
    }
    links
}

async fn maccms_search(client: &HttpClient, keyword: &str, base: &str, id_re: &Regex, name: &str, only_uc: bool) -> Result<Vec<SearchResult>> {
    let url = format!("{}/index.php/vod/search/wd/{}.html", base, q(keyword));
    let body = client.get_text(&url, Some(browser_headers(Some(&format!("{}/", base))))).await?;
    let items = maccms_parse_list(&body, id_re);
    let mut futs = Vec::new();
    for (id, title) in items {
        let du = format!("{}/index.php/vod/detail/id/{}.html", base, id);
        let c = client.clone();
        let h = browser_headers(Some(&format!("{}/", base)));
        futs.push(async move { let b = c.get_text(&du, Some(h)).await.ok(); (id, title, b) });
    }
    let mut out = Vec::new();
    for (id, title, dbody) in futures::future::join_all(futs).await {
        let dbody = match dbody { Some(b) => b, None => continue };
        let links = maccms_detail_links(&dbody, only_uc);
        if links.is_empty() { continue; }
        let mut r = result(name, &id, &title); r.links = links; out.push(r);
    }
    Ok(out)
}

pub struct Huban;
static HUBAN_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/id/(\d+)").unwrap());
#[async_trait]
impl SearchPlugin for Huban {
    fn name(&self) -> &'static str { "huban" }
    fn priority(&self) -> i32 { 2 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        maccms_search(client, keyword, "http://103.45.162.207:20720", &HUBAN_ID_RE, "huban", false).await
    }
}
pub struct Shandian;
static SHANDIAN_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/vod/detail/id/(\d+)\.html").unwrap());
#[async_trait]
impl SearchPlugin for Shandian {
    fn name(&self) -> &'static str { "shandian" }
    fn priority(&self) -> i32 { 2 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        maccms_search(client, keyword, "http://1.95.79.193", &SHANDIAN_ID_RE, "shandian", true).await
    }
}

// ============================================================
// jutoushe：https://1.star2.cn/search/?keyword=（两步）
// ============================================================
pub struct Jutoushe;
static JTS_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/([^/]+)/(\d+)\.html").unwrap());
static JTS_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"【([^】]+)】").unwrap());
fn jts_parse_list(body: &str) -> Vec<(String, String, String)> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for li in sel(&doc, "ul.erx-list li.item") {
        let a = match sel(&li, ".a a.main").into_iter().next() { Some(e) => e, None => continue };
        let title = a.text().collect::<String>().trim().to_string();
        let href = a.value().attr("href").unwrap_or("");
        if title.is_empty() || href.is_empty() { continue; }
        let detail = if href.starts_with("http") { href.to_string() } else { format!("https://1.star2.cn{}", href) };
        out.push((detail, title, first_text(&li, ".i span.time")));
    }
    out
}
fn jts_detail_links(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let mut links = Vec::new();
    for da in sel(&doc, ".dlipp-cont-bd a.dlipp-dl-btn") {
        let u = da.value().attr("href").unwrap_or("").to_string();
        if u.is_empty() { continue; }
        let ty = determine_type(&u);
        if ty == "others" { continue; }
        let mut pass = String::new();
        if u.contains("pan.baidu.com") && u.contains("pwd=") { pass = url_password(&u); }
        if links.iter().any(|l: &Link| l.url == u) { continue; }
        links.push(Link { r#type: ty.to_string(), url: u, password: pass, datetime: None, work_title: None });
    }
    links
}
#[async_trait]
impl SearchPlugin for Jutoushe {
    fn name(&self) -> &'static str { "jutoushe" }
    fn priority(&self) -> i32 { 1 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let body = client.get_text(&format!("https://1.star2.cn/search/?keyword={}", q(keyword)), Some(browser_headers(Some("https://1.star2.cn/")))).await?;
        let items = jts_parse_list(&body);
        let mut futs = Vec::new();
        for (detail, title, time) in items {
            let c = client.clone();
            futs.push(async move { let b = c.get_text(&detail, Some(browser_headers(Some("https://1.star2.cn/")))).await.ok(); (detail, title, time, b) });
        }
        let mut out = Vec::new();
        for (detail, title, time, dbody) in futures::future::join_all(futs).await {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = jts_detail_links(&dbody);
            if links.is_empty() { continue; }
            let id = JTS_ID_RE.captures(&detail).and_then(|c| c.get(2)).map(|m| m.as_str().to_string()).unwrap_or_else(|| detail.replace('/', "_"));
            let mut r = result("jutoushe", &id, &title);
            r.content = format!("剧透社影视资源：{} | {}", title, time);
            let tags: Vec<String> = JTS_TAG_RE.captures_iter(&title).filter_map(|c| c.get(1)).map(|m| m.as_str().to_string()).collect();
            r.tags = if tags.is_empty() { vec!["影视资源".into()] } else { tags };
            r.links = links; out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// xinjuc：https://www.xinjuc.com/?s=（两步，仅百度）
// ============================================================
pub struct Xinjuc;
static XJC_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/(\d+)\.html").unwrap());
static XJC_BAIDU_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://pan\.baidu\.com/s/[0-9a-zA-Z_\-]{10,}(?:\?pwd=[0-9a-zA-Z]+)?").unwrap());
static XJC_PWD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"提取码[:：]\s*([a-zA-Z0-9]{4})").unwrap());
fn xjc_parse_list(body: &str) -> Vec<(String, String)> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for art in sel(&doc, "div.row-xs.post-list article.post-item") {
        let a = match sel(&art, "div.post-image a").into_iter().next() { Some(e) => e, None => continue };
        let title = first_text(&art, "h5.post-title a");
        let href = a.value().attr("href").unwrap_or("");
        if title.is_empty() || href.is_empty() { continue; }
        let detail = if href.starts_with("http") { href.to_string() }
            else if href.starts_with('/') { format!("https://www.xinjuc.com{}", href) }
            else { format!("https://www.xinjuc.com/{}", href) };
        out.push((detail, title));
    }
    out
}
fn xjc_detail(body: &str) -> Option<Vec<Link>> {
    let doc = Html::parse_document(body);
    if sel(&doc, "div.article-content").into_iter().next().is_none() { return None; }
    let mut links = Vec::new();
    for m in XJC_BAIDU_RE.find_iter(body) {
        let u = m.as_str();
        if links.iter().any(|l: &Link| l.url == u) { continue; }
        let mut pass = url_password(u);
        if pass.is_empty() { pass = XJC_PWD_RE.captures(body).and_then(|c| c.get(1)).map(|x| x.as_str().to_string()).unwrap_or_default(); }
        links.push(Link { r#type: "baidu".into(), url: u.to_string(), password: pass, datetime: None, work_title: None });
    }
    if links.is_empty() { None } else { Some(links) }
}
#[async_trait]
impl SearchPlugin for Xinjuc {
    fn name(&self) -> &'static str { "xinjuc" }
    fn priority(&self) -> i32 { 2 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let body = client.get_text(&format!("https://www.xinjuc.com/?s={}", q(keyword)), Some(browser_headers(Some("https://www.xinjuc.com")))).await?;
        let items = xjc_parse_list(&body);
        let mut futs = Vec::new();
        for (detail, title) in items {
            let c = client.clone();
            futs.push(async move { let b = c.get_text(&detail, Some(browser_headers(Some("https://www.xinjuc.com")))).await.ok(); (detail, title, b) });
        }
        let mut out = Vec::new();
        for (detail, title, dbody) in futures::future::join_all(futs).await {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = match xjc_detail(&dbody) { Some(l) => l, None => continue };
            let id = XJC_ID_RE.captures(&detail).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_else(|| format!("{:x}", md5::Md5::digest(detail.as_bytes())));
            let mut r = result("xinjuc", &id, &title); r.links = links; out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// u3c3：动态 search2 token + 搜索页（仅 magnet），跳过 Service 过滤
// ============================================================
pub struct U3c3;
const U3C3_BASE: &str = "https://u3c3u3c3.u3c3u3c3u3c3.com";
static U3C3_TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"var\s+nmefafej\s*=\s*"([^"]+)""#).unwrap());
fn u3c3_extract_token(home: &str) -> String {
    for line in home.lines() {
        let l = line.trim();
        if l.starts_with("//") { continue; }
        if l.contains("nmefafej") && l.contains('"') {
            if let Some(c) = U3C3_TOKEN_RE.captures(l) {
                let v = c.get(1).map(|m| m.as_str()).unwrap_or("");
                if v.len() > 5 { return v.to_string(); }
            }
            if let Some(s) = l.find('"') {
                if let Some(e) = l[s + 1..].find('"') {
                    let v = &l[s + 1..s + 1 + e];
                    if v.len() > 5 { return v.to_string(); }
                }
            }
        }
    }
    String::new()
}
fn u3c3_hash(s: &str) -> i64 {
    let mut h: i64 = 0;
    for ch in s.chars() { h = h.wrapping_mul(31).wrapping_add(ch as i64); }
    h.abs()
}
fn u3c3_parse(body: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for tr in sel(&doc, "tbody tr.default") {
        let tds = sel(&tr, "td");
        if tds.len() < 5 { continue; }
        if tds.get(1).map(|e| e.text().collect::<String>()).unwrap_or_default().contains("[置顶]") { continue; }
        let title = match sel(&tr, "td:nth-child(2) a").into_iter().next() {
            Some(a) => crate::plugins::helpers::strip_tags(&a.text().collect::<String>()), None => continue };
        let magnet = sel(&tr, "td:nth-child(3) a[href^='magnet:']").into_iter().next()
            .and_then(|a| a.value().attr("href").map(|s| s.to_string()));
        let magnet = match magnet { Some(m) => m, None => continue };
        let size = tds.get(3).map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let time = tds.get(4).map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let cat = sel(&tr, "td:nth-child(1) a").into_iter().next()
            .and_then(|a| a.value().attr("title").map(|s| s.to_string())).unwrap_or_default();
        let hash = u3c3_hash(&format!("u3c3-{}-{}", title, size));
        let mut r = result("u3c3", &hash.to_string(), &title);
        r.content = format!("分类: {} | 大小: {} | 时间: {}", cat, size, time);
        r.tags = vec!["种子".into(), "磁力链接".into()];
        r.links.push(Link { r#type: "magnet".into(), url: magnet, password: String::new(), datetime: None, work_title: None });
        out.push(r);
    }
    out
}
#[async_trait]
impl SearchPlugin for U3c3 {
    fn name(&self) -> &'static str { "u3c3" }
    fn priority(&self) -> i32 { 5 }
    fn skip_service_filter(&self) -> bool { true }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let home = client.get_text(U3C3_BASE, Some(browser_headers(None))).await?;
        let token = u3c3_extract_token(&home);
        let body = client.get_text(&format!("{}/?search2={}&search={}", U3C3_BASE, q(&token), q(keyword)), Some(browser_headers(Some(&format!("{}/", U3C3_BASE))))).await?;
        Ok(u3c3_parse(&body))
    }
}
