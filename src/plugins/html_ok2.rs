//! 两步 HTML 插件（已验证可用）：mizixing / ypfxw
//! 两个插件结构高度相似：列表页 → 详情页 → 提取网盘链接。解析同步化。

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
use scraper::Html;

fn q(s: &str) -> String { crate::util::text::url_encode(s) }

fn result(name: &str, uid: &str, title: &str) -> SearchResult {
    SearchResult {
        unique_id: format!("{}-{}", name, uid),
        title: title.to_string(),
        datetime: Utc::now(),
        channel: String::new(),
        ..Default::default()
    }
}

static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s<>"']+"#).unwrap());

/// 通用：从正文节点集合 + 纯文本中提取网盘链接（带密码关联）
fn extract_links_from_doc(doc: &Html, text_fallback: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let mut push = |url: &str, ctx: &str| {
        let url = url.trim_end_matches(|c: char| matches!(c, ',' | '。' | '，' | ';' | ')' | '）' | '>'));
        if url.is_empty() { return; }
        let ty = determine_type(url);
        if ty == "others" && !url.starts_with("magnet:") && !url.starts_with("ed2k:") { return; }
        if out.iter().any(|l: &Link| l.url == url) { return; }
        let pass = { let p = url_password(url); if p.is_empty() { password_near(ctx) } else { p } };
        out.push(Link { r#type: ty.to_string(), url: url.to_string(), password: pass, datetime: None, work_title: None });
    };
    for a in sel(doc, "a[href]") {
        let href = a.value().attr("href").unwrap_or("");
        let parent_text = a.parent()
            .and_then(|p| scraper::ElementRef::wrap(p))
            .map(|p| p.text().collect::<String>()).unwrap_or_default();
        let ctx = format!("{} {} {}", a.text().collect::<String>(),
            a.value().attr("title").unwrap_or(""), parent_text);
        push(href, &ctx);
    }
    for m in URL_RE.find_iter(text_fallback) { push(m.as_str(), text_fallback); }
    out
}

fn context_around(text: &str, url: &str) -> String {
    if let Some(pos) = text.find(url) {
        let start = pos.saturating_sub(80);
        let end = (pos + url.len() + 80).min(text.len());
        // UTF-8 安全切片
        let s = crate::util::netdisk::floor_char_boundary(text, start);
        let e = crate::util::netdisk::ceil_char_boundary(text, end);
        text[s..e].to_string()
    } else { String::new() }
}

// ============================================================
// mizixing：两步 HTML，https://mizixing.com/?s=
// 列表 article.excerpt，详情 h2 a[href]；正文 article.article-content 等
// 唯一ID = mizixing-{crc32(detailURL)}
// ============================================================

pub struct Mizixing;

fn mizixing_parse_list(body: &str) -> Vec<(String, String, String)> {
    // (detail_url, title, summary)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for item in sel(&doc, "article.excerpt") {
        let a = sel(&item, "h2 a").into_iter().next().or_else(|| sel(&item, "h2").into_iter().next());
        let (title, href) = match a {
            Some(e) => (e.text().collect::<String>().trim().to_string(),
                        e.value().attr("href").unwrap_or("").to_string()),
            None => continue,
        };
        if href.is_empty() { continue; }
        let detail = if href.starts_with("http") { href } else { format!("https://mizixing.com{}", href) };
        let summary = first_text(&item, "p.note");
        out.push((detail, title, summary));
    }
    out.truncate(12);
    out
}

fn mizixing_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    // 选正文容器
    let content = sel(&doc, "article.article-content").into_iter().next()
        .or_else(|| sel(&doc, ".article-content").into_iter().next())
        .or_else(|| sel(&doc, ".entry-content").into_iter().next());
    let full_text = doc.root_element().text().collect::<String>();
    match content {
        Some(c) => {
            let txt = c.text().collect::<String>();
            let mut links = extract_links_from_doc(&Html::parse_fragment(&c.html()), &txt);
            // 全文兜底
            if links.is_empty() { links = extract_links_from_doc(&doc, &full_text); }
            links
        }
        None => extract_links_from_doc(&doc, &full_text),
    }
}

#[async_trait]
impl SearchPlugin for Mizixing {
    fn name(&self) -> &'static str { "mizixing" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = format!("https://mizixing.com/?s={}", q(keyword));
        let body = client.get_text(&url, Some(browser_headers(Some("https://mizixing.com")))).await?;
        let items = mizixing_parse_list(&body);
        let mut futs = Vec::new();
        for (detail, title, summary) in items {
            let c = client.clone();
            futs.push(async move { let b = c.get_text(&detail, Some(browser_headers(Some("https://mizixing.com")))).await.ok(); (detail, title, summary, b) });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (detail, title, summary, dbody) in details {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = mizixing_parse_detail(&dbody);
            if links.is_empty() { continue; }
            let crc = crc32fast::hash(detail.as_bytes());
            let mut r = result("mizixing", &crc.to_string(), &title);
            r.content = summary;
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// ypfxw：两步 HTML，https://ypfxw.com/search.php?q=
// 列表 div.list ul>li，详情 div.imgr h2 a[href]；正文 .article_content
// 唯一ID = ypfxw-{postID}，postID 来自 /post/(\d+).html
// ============================================================

pub struct Ypfxw;
static YPFXW_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/post/(\d+)\.html").unwrap());

fn ypfxw_parse_list(body: &str) -> Vec<(String, String, String)> {
    // (detail_url, title, meta)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for li in sel(&doc, "div.list ul > li") {
        let a = match sel(&li, "div.imgr h2 a").into_iter().next() { Some(e) => e, None => continue };
        let title = a.text().collect::<String>().trim().to_string();
        let href = a.value().attr("href").unwrap_or("").to_string();
        if href.is_empty() { continue; }
        let summary = first_text(&li, "div.imgr p");
        let category = first_text(&li, ".info span");
        out.push((href, title, format!("{} | {}", category, summary)));
    }
    out
}

fn ypfxw_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let content = sel(&doc, ".article_content").into_iter().next();
    let full_text = doc.root_element().text().collect::<String>();
    let mut links = match content {
        Some(c) => {
            let txt = c.text().collect::<String>();
            let mut l = extract_links_from_doc(&Html::parse_fragment(&c.html()), &txt);
            if l.is_empty() { l = extract_links_from_doc(&doc, &full_text); }
            l
        }
        None => extract_links_from_doc(&doc, &full_text),
    };
    // 用上下文补充密码
    for l in &mut links {
        if l.password.is_empty() {
            l.password = password_near(&context_around(&full_text, &l.url));
        }
    }
    links
}

#[async_trait]
impl SearchPlugin for Ypfxw {
    fn name(&self) -> &'static str { "ypfxw" }
    fn priority(&self) -> i32 { 2 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = format!("https://ypfxw.com/search.php?q={}", q(keyword));
        let body = client.get_text(&url, Some(browser_headers(Some("https://ypfxw.com/")))).await?;
        let items = ypfxw_parse_list(&body);
        let mut futs = Vec::new();
        for (detail, title, meta) in items {
            let c = client.clone();
            futs.push(async move { let b = c.get_text(&detail, Some(browser_headers(Some("https://ypfxw.com/")))).await.ok(); (detail, title, meta, b) });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (detail, title, meta, dbody) in details {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = ypfxw_parse_detail(&dbody);
            if links.is_empty() { continue; }
            let id = YPFXW_ID_RE.captures(&detail).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
                .unwrap_or_else(|| format!("{:x}", crc32fast::hash(detail.as_bytes())));
            let mut r = result("ypfxw", &id, &title);
            r.content = meta;
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}

/// 供 yulinshufa 复用的通用链接提取（导出）
pub(crate) fn extract_links_generic(doc: &Html, text: &str) -> Vec<Link> {
    extract_links_from_doc(doc, text)
}
pub(crate) fn context_around_generic(text: &str, url: &str) -> String { context_around(text, url) }
