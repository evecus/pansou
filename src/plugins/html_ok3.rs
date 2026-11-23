//! HTML 插件（已验证可用）：yulinshufa(GBK) / panwiki(Discuz 三步)
//! 解析同步化。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{determine_type, first_text, password_near, sel, url_password};
use crate::plugins::html_ok2::{context_around_generic, extract_links_generic};
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

// ============================================================
// yulinshufa：GBK 站点，http://www.yulinshufa.cn/plus/search.php?q=
// 关键词 GBK 百分号编码；响应 GBK 解码；两步 HTML
// ============================================================

pub struct Yulinshufa;
const YULIN_BASE: &str = "http://www.yulinshufa.cn";

/// 关键词 → GBK 百分号编码
fn gbk_query_escape(s: &str) -> String {
    let (gbk, _, _) = encoding_rs::GBK.encode(s);
    let mut out = String::new();
    for &b in gbk.as_ref() {
        match b {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// GBK 字节 → UTF-8 字符串
fn gbk_decode(bytes: &[u8]) -> String {
    let (s, _, _) = encoding_rs::GBK.decode(bytes);
    s.into_owned()
}

fn yulin_normalize(href: &str) -> String {
    if href.starts_with("//") { format!("http:{}", href) }
    else if href.starts_with("http") { href.to_string() }
    else if href.starts_with('/') { format!("{}{}", YULIN_BASE, href) }
    else if href.starts_with("./") { format!("{}{}", YULIN_BASE, href.trim_start_matches('.')) }
    else { format!("{}/{}", YULIN_BASE, href) }
}

fn yulin_parse_list(body: &str) -> Vec<(String, String, String)> {
    // (detail_url, title, meta)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for li in sel(&doc, "div.main-list-con ul.main-list > li") {
        let a = match sel(&li, "div.list-con p.s-title a").into_iter().next() { Some(e) => e, None => continue };
        let title = a.text().collect::<String>().trim().to_string();
        let href = a.value().attr("href").unwrap_or("");
        if href.is_empty() { continue; }
        let detail = yulin_normalize(href);
        let desc = first_text(&li, "div.list-con p.s-desc");
        let cat = first_text(&li, "div.list-con div.s-ext span.item a");
        out.push((detail, title, format!("{} | {}", cat, desc)));
    }
    out.truncate(15);
    out
}

static YULIN_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/xz/(\d+)/?").unwrap());

fn yulin_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    if sel(&doc, "div.content").into_iter().next().is_none() { return Vec::new(); }
    let full = doc.root_element().text().collect::<String>();
    let mut links = extract_links_generic(&doc, &full);
    // 关联密码（ DOM 顺序上链接后面的密码）
    for l in &mut links {
        if l.password.is_empty() {
            let ctx = context_around_generic(&full, &l.url);
            static PW: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(提取码|密码)[:：\s]*([0-9a-zA-Z]{3,8})").unwrap());
            l.password = PW.captures(&ctx).and_then(|c| c.get(2)).map(|m| m.as_str().to_string()).unwrap_or_default();
        }
    }
    links
}

#[async_trait]
impl SearchPlugin for Yulinshufa {
    fn name(&self) -> &'static str { "yulinshufa" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let kw = ext.get("title_en").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(keyword);
        let url = format!("{}/plus/search.php?q={}", YULIN_BASE, gbk_query_escape(kw));
        let mut headers = browser_headers(Some(&format!("{}/", YULIN_BASE)));
        headers.insert("cache-control", reqwest::header::HeaderValue::from_static("max-age=0"));
        let bytes = client.get_bytes(&url, Some(headers)).await?;
        let body = gbk_decode(&bytes);
        let items = yulin_parse_list(&body);
        let mut futs = Vec::new();
        for (detail, title, meta) in items {
            let c = client.clone();
            futs.push(async move {
                let b = c.get_bytes(&detail, Some(browser_headers(Some(&format!("{}/", YULIN_BASE))))).await.ok();
                (detail, title, meta, b)
            });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (detail, title, meta, dbody) in details {
            let dbody = match dbody { Some(b) => b, None => continue };
            let dtext = gbk_decode(&dbody);
            let links = yulin_parse_detail(&dtext);
            if links.is_empty() { continue; }
            let id = YULIN_ID_RE.captures(&detail).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
                .unwrap_or_else(|| detail.replace('/', "-").trim_matches('-').to_string());
            let mut r = result("yulinshufa", &id, &title);
            r.content = meta;
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// panwiki：Discuz 论坛三步，主域 panwiki.com / 备域 pan666.net
// 搜索 /search.php?mod=forum&srchtxt=&searchsubmit=yes&orderby=lastpost
// 禁用重定向取 Location → 构造真实搜索URL → 列表 → 详情
// ============================================================

pub struct Panwiki;
static PANWIKI_SEARCHID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"searchid=(\d+)").unwrap());
static PANWIKI_TID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"tid=(\d+)").unwrap());
static PANWIKI_AD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(【[^】]*(?:论坛|网站|\.com|\.net|\.cn)[^】]*】|\[[^\]]*(?:论坛|网站|\.com|\.net|\.cn)[^\]]*\])").unwrap());

fn panwiki_clean_title(s: &str) -> String {
    PANWIKI_AD_RE.replace_all(s, "").trim().to_string()
}

fn panwiki_search_headers(base: &str) -> reqwest::header::HeaderMap {
    browser_headers(Some(&format!("{}/", base)))
}

fn panwiki_parse_list(body: &str) -> Vec<(String, String, String)> {
    // (detail_url, title, meta)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for li in sel(&doc, ".slst ul li.pbw") {
        let a = match sel(&li, "h3.xs3 a").into_iter().next() { Some(e) => e, None => continue };
        let title = panwiki_clean_title(&a.text().collect::<String>());
        let href = a.value().attr("href").unwrap_or("").to_string();
        if href.is_empty() { continue; }
        let ps = sel(&li, "p");
        let summary = ps.get(1).map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let last_p = sel(&li, "p").into_iter().last();
        let mut meta = summary;
        if let Some(lp) = last_p {
            let spans = sel(&lp, "span");
            let stats = lp.text().collect::<String>().trim().to_string();
            if !stats.is_empty() { meta = format!("{} | {}", meta, stats); }
            let _ = spans;
        }
        let tid = PANWIKI_TID_RE.captures(&href).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
            .unwrap_or_else(|| format!("{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        // detail url 用 tid 构造（稍后 base 拼接）
        out.push((tid, title, meta));
    }
    out
}

fn panwiki_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let content = sel(&doc, ".t_f[id^='postmessage_']").into_iter().next()
        .or_else(|| sel(&doc, ".t_msgfont, .plhin, .message, [id^='postmessage_']").into_iter().next());
    let full = doc.root_element().text().collect::<String>();
    let mut links = match content {
        Some(c) => {
            let txt = c.text().collect::<String>();
            let mut l = extract_links_generic(&Html::parse_fragment(&c.html()), &txt);
            if l.is_empty() { l = extract_links_generic(&doc, &full); }
            l
        }
        None => extract_links_generic(&doc, &full),
    };
    // panwiki 有"丨"分组格式，补充从文本按行提取
    for l in &mut links {
        if l.password.is_empty() {
            let ctx = context_around_generic(&full, &l.url);
            static PW: Lazy<Regex> = Lazy::new(|| Regex::new(r"(提取码|密码|pwd|password)[：:=]\s*([A-Za-z0-9]+)").unwrap());
            l.password = PW.captures(&ctx).and_then(|c| c.get(2)).map(|m| m.as_str().to_string()).unwrap_or_default();
        }
    }
    links
}

/// 禁用重定向请求，返回 (最终URL或Location, body)
async fn panwiki_no_redirect(client: &HttpClient, url: &str, base: &str) -> (Option<String>, Option<String>) {
    let resp = client.client.get(url).headers(panwiki_search_headers(base)).send().await.ok();
    let resp = match resp { Some(r) => r, None => return (None, None) };
    let location = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    match location {
        Some(loc) => {
            let real = if loc.starts_with("http") { loc } else { format!("{}/{}", base, loc.trim_start_matches('/')) };
            (Some(real), None)
        }
        None => {
            let body = resp.text().await.ok();
            (None, body)
        }
    }
}

#[async_trait]
impl SearchPlugin for Panwiki {
    fn name(&self) -> &'static str { "panwiki" }
    fn priority(&self) -> i32 { 3 }
    fn skip_service_filter(&self) -> bool { true }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let bases = ["https://www.panwiki.com", "https://pan666.net"];
        let mut base = bases[0];
        // 第一步：初始搜索取 Location
        let init = format!("{}/search.php?mod=forum&srchtxt={}&searchsubmit=yes&orderby=lastpost", base, q(keyword));
        let (loc, direct_body) = panwiki_no_redirect(client, &init, base).await;
        // 第二步：构造真实搜索 URL 并请求列表
        let search_body = if let Some(real) = loc {
            // 提取 searchid
            if let Some(c) = PANWIKI_SEARCHID_RE.captures(&real) {
                let sid = c.get(1).map(|m| m.as_str()).unwrap_or("");
                let real2 = format!("{}/search.php?mod=forum&searchid={}&orderby=lastpost&ascdesc=desc&searchsubmit=yes", base, sid);
                client.get_text(&real2, Some(panwiki_search_headers(base))).await.unwrap_or_default()
            } else {
                client.get_text(&real, Some(panwiki_search_headers(base))).await.unwrap_or_default()
            }
        } else if let Some(b) = direct_body { b }
        else {
            // 主域失败，切换备域重试一次
            base = bases[1];
            let init2 = format!("{}/search.php?mod=forum&srchtxt={}&searchsubmit=yes&orderby=lastpost", base, q(keyword));
            let (loc2, body2) = panwiki_no_redirect(client, &init2, base).await;
            if let Some(real) = loc2 { client.get_text(&real, Some(panwiki_search_headers(base))).await.unwrap_or_default() }
            else { body2.unwrap_or_default() }
        };
        let items = panwiki_parse_list(&search_body);
        if items.is_empty() { return Ok(Vec::new()); }
        // 第三步：并发抓详情
        let mut futs = Vec::new();
        for (tid, title, meta) in items {
            let url = format!("{}/forum.php?mod=viewthread&tid={}", base, tid);
            let c = client.clone();
            let h = panwiki_search_headers(base);
            futs.push(async move { let b = c.get_text(&url, Some(h)).await.ok(); (tid, title, meta, b) });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (tid, title, meta, dbody) in details {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = panwiki_parse_detail(&dbody);
            if links.is_empty() { continue; }
            let mut r = result("panwiki", &tid, &title);
            r.content = meta;
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}
