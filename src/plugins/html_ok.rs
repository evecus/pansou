//! HTML 两步抓取类插件（测速 OK）：aikanzy / xiaozhang / pianku / quarksoo
//! 所有 HTML 解析抽成同步函数，避免 scraper::Html 跨 await 导致 future 非 Send。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{clean_html, determine_type, password_near, sel, url_password};
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
    SearchResult {
        unique_id: format!("{}-{}", name, uid),
        title: title.to_string(),
        datetime: Utc::now(),
        channel: String::new(),
        ..Default::default()
    }
}

// ============================================================
// quarksoo：单步 HTML 正则，https://quarksoo.cc/search.php?q=
// <tr><td>标题</td><td><a href=链接> → 仅夸克
// ============================================================

pub struct Quarksoo;

fn quarksoo_parse(body: &str, keyword: &str) -> Vec<SearchResult> {
    use std::collections::HashSet;
    let doc = Html::parse_document(body);
    let lower = keyword.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tr in sel(&doc, "tr") {
        // 标题：优先 .drama-cell span，否则第一个 td 文本
        let title = sel(&tr, ".drama-cell span").into_iter().next()
            .map(|e| e.text().collect::<String>())
            .or_else(|| sel(&tr, "td").into_iter().next().map(|e| e.text().collect::<String>()))
            .unwrap_or_default().trim().to_string();
        let url = sel(&tr, "a[href]").into_iter().next()
            .and_then(|a| a.value().attr("href").map(|s| s.trim().to_string())).unwrap_or_default();
        if title.is_empty() || title.contains("剧名") || title.contains("网盘链接") { continue; }
        if !url.contains("pan.qoark.cn") && !url.contains("pan.quark.cn") { continue; }
        let lt = title.to_lowercase();
        if !words.iter().all(|w| lt.contains(w)) { continue; }
        if !seen.insert(url.clone()) { continue; }
        let key = format!("{}|{}", title, url);
        let short = format!("{:x}", md5::Md5::digest(key.as_bytes()));
        let mut r = result("quarksoo", &short[..16], &title);
        r.links.push(Link {
            r#type: "quark".to_string(), url,
            password: String::new(), datetime: None, work_title: None,
        });
        out.push(r);
    }
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

#[async_trait]
impl SearchPlugin for Quarksoo {
    fn name(&self) -> &'static str { "quarksoo" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = format!("https://quarksoo.cc/search.php?q={}", q(keyword));
        let body = client.get_text(&url, Some(browser_headers(Some("https://quarksoo.cc/")))).await?;
        Ok(quarksoo_parse(&body, keyword))
    }
}

// ============================================================
// aikanzy：两步 HTML，https://www.aikanzy.com/search?word=&molds=article
// 列表 article.post-list.contt.blockimg → 详情首个 a[href]
// 链接仅 quark/uc/baidu/xunlei 4 类，密码 pwd=XXXX（恰好4位）
// ============================================================

pub struct Aikanzy;

fn aikanzy_type(url: &str) -> &'static str {
    let s = url.to_lowercase();
    if s.contains("pan.quark.cn") { "quark" }
    else if s.contains("drive.uc.cn") { "uc" }
    else if s.contains("pan.baidu.com") { "baidu" }
    else if s.contains("pan.xunlei.com") { "xunlei" }
    else { "" }
}

fn aikanzy_password(s: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"pwd=([^#&]{4})").unwrap());
    RE.captures(s).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default()
}

fn aikanzy_parse_list(body: &str, keyword: &str) -> Vec<(String, String, String, String, String)> {
    // 返回 (id, detail_url, title, meta_info, summary)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    static ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/([a-z]+)/(\d+)\.html").unwrap());
    for item in sel(&doc, "article.post-list.contt.blockimg") {
        let detail = sel(&item, "a[href]").into_iter().next()
            .and_then(|e| e.value().attr("href").map(|s| s.to_string())).unwrap_or_default();
        if detail.is_empty() { continue; }
        let title = clean_html(&sel(&item, "header.entry-header span.entry-title a").into_iter().next()
            .map(|e| e.text().collect::<String>()).unwrap_or_default());
        let category = sel(&item, "div.entry-meta > a").into_iter().next()
            .map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let date = sel(&item, "time").into_iter().next()
            .map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let views_text = sel(&item, "div.entry-meta").into_iter().next()
            .map(|e| e.text().collect::<String>()).unwrap_or_default();
        static VIEW_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d+)\s*阅读").unwrap());
        let views = VIEW_RE.captures(&views_text).and_then(|c| c.get(1)).map(|m| m.as_str()).unwrap_or("0");
        let summary = sel(&item, "div.entry-summary.ss p").into_iter().next()
            .map(|e| clean_html(&e.text().collect::<String>())).unwrap_or_default();
        let id = ID_RE.captures(&detail).and_then(|c| c.get(2)).map(|m| m.as_str().to_string())
            .unwrap_or_else(|| format!("{:x}", md5::Md5::digest(detail.as_bytes())));
        let mut meta = Vec::new();
        if !category.is_empty() { meta.push(category.clone()); }
        if !date.is_empty() { meta.push(date.clone()); }
        meta.push(format!("{}阅读", views));
        let info = format!("{}|{}|{}", meta.join(" | "), summary, category);
        out.push((id, detail, title, info, summary));
    }
    let _ = keyword;
    out
}

fn aikanzy_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let mut links = Vec::new();
    // 方法1：选择器
    let anchors = sel(&doc, "a[href*='pan.quark.cn'], a[href*='drive.uc.cn'], a[href*='pan.baidu.com'], a[href*='pan.xunlei.com']");
    for a in anchors {
        let href = a.value().attr("href").unwrap_or("").to_string();
        let ty = aikanzy_type(&href);
        if ty.is_empty() { continue; }
        let ctx = format!("{} {}", a.text().collect::<String>(), href);
        let pass = aikanzy_password(&ctx);
        if links.iter().any(|l: &Link| l.url == href) { continue; }
        links.push(Link { r#type: ty.to_string(), url: href, password: pass, datetime: None, work_title: None });
    }
    if !links.is_empty() { return links; }
    // 方法2：正则兜底
    static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
        r#"https?://(?:pan\.quark\.cn|drive\.uc\.cn|pan\.baidu\.com|pan\.xunlei\.com)/[0-9a-zA-Z_/-]+"#
    ).unwrap());
    for m in URL_RE.find_iter(body) {
        let href = m.as_str().to_string();
        let ty = aikanzy_type(&href);
        if ty.is_empty() { continue; }
        if links.iter().any(|l: &Link| l.url == href) { continue; }
        links.push(Link {
            r#type: ty.to_string(), url: href, password: aikanzy_password(body),
            datetime: None, work_title: None,
        });
    }
    links
}

#[async_trait]
impl SearchPlugin for Aikanzy {
    fn name(&self) -> &'static str { "aikanzy" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = format!("https://www.aikanzy.com/search?word={}&molds=article", q(keyword));
        let body = client.get_text(&url, Some(browser_headers(Some("https://www.aikanzy.com/")))).await?;
        let items = aikanzy_parse_list(&body, keyword);
        let mut futs = Vec::new();
        for (id, detail, title, info, _summary) in items {
            let c = client.clone();
            futs.push(async move {
                let dbody = c.get_text(&detail, Some(browser_headers(Some("https://www.aikanzy.com/")))).await.ok();
                (id, title, info, dbody)
            });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (id, title, info, dbody) in details {
            let links = match dbody { Some(b) => aikanzy_parse_detail(&b), None => Vec::new() };
            if links.is_empty() { continue; }
            let mut r = result("aikanzy", &id, &title);
            r.content = info.split('|').next().unwrap_or("").to_string();
            r.tags.push(info.split('|').last().unwrap_or("").to_string());
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// xiaozhang：两步 HTML + 302 重定向，https://xzys.fun/search.html?keyword=
// 列表 .list-boxes → 详情(禁跟随重定向取 Location → 拼真实URL → 再请求)
// 链接仅 p 下 a[href]，域名白名单；type 含 jianguo/cowtransfer/weidian
// ============================================================

pub struct Xiaozhang;

fn xiaozhang_type(url: &str) -> &'static str {
    let s = url.to_lowercase();
    if s.contains("pan.baidu.com") { "baidu" }
    else if s.contains("pan.quark.cn") { "quark" }
    else if s.contains("www.aliyundrive.com") || s.contains("www.alipan.com") { "aliyun" }
    else if s.contains("115.com") { "115" }
    else if s.contains("cloud.189.cn") { "tianyi" }
    else if s.contains("pan.xunlei.com") { "xunlei" }
    else if s.contains("www.123pan.com") { "123" }
    else if s.contains("www.jianguoyun.com") { "jianguo" }
    else if s.contains("cowtransfer.com") { "cowtransfer" }
    else if s.contains("weidian.com") { "weidian" }
    else { "" }
}

fn xiaozhang_parse_list(body: &str) -> Vec<(String, String, String, String)> {
    // (id, detail_url, title, content)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    static ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/subject/(\d+)\.html").unwrap());
    static TIME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap());
    for item in sel(&doc, ".list-boxes") {
        let a = sel(&item, "a.text_title_p").into_iter().next();
        let (title, href) = match a {
            Some(e) => (e.text().collect::<String>().trim().to_string(),
                        e.value().attr("href").unwrap_or("").to_string()),
            None => continue,
        };
        if href.is_empty() { continue; }
        let detail = if href.starts_with("http") { href.clone() } else { format!("https://xzys.fun{}", href) };
        let content = sel(&item, "p.text_p").into_iter().next()
            .map(|e| e.text().collect::<String>().replace("&nbsp;", " ").trim().to_string()).unwrap_or_default();
        let time_text = sel(&item, ".list-actions span").into_iter().next()
            .map(|e| e.text().collect::<String>()).unwrap_or_default();
        let id = ID_RE.captures(&detail).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
            .unwrap_or_else(|| format!("{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let _ = TIME_RE;
        out.push((id, detail, title, content));
    }
    out
}

fn xiaozhang_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let mut links = Vec::new();
    for p in sel(&doc, "p") {
        for a in sel(&p, "a[href]") {
            let href = a.value().attr("href").unwrap_or("").to_string();
            let ty = xiaozhang_type(&href);
            if ty.is_empty() { continue; }
            if links.iter().any(|l: &Link| l.url == href) { continue; }
            let ptext = p.text().collect::<String>();
            let mut pass = String::new();
            if ptext.contains("提取码") || ptext.contains("密码") {
                static PW_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:提取码|密码)[：:]?\s*([a-zA-Z0-9]+)").unwrap());
                pass = PW_RE.captures(&ptext).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
            }
            if pass.is_empty() { pass = url_password(&href); }
            links.push(Link { r#type: ty.to_string(), url: href, password: pass, datetime: None, work_title: None });
        }
    }
    links
}

/// 禁跟随重定向取 Location
async fn xiaozhang_location(client: &HttpClient, url: &str) -> Option<String> {
    let resp = client.client.get(url)
        .headers(browser_headers(Some("https://xzys.fun")))
        .send().await.ok()?;
    let loc = resp.headers().get(reqwest::header::LOCATION)?.to_str().ok()?.to_string();
    Some(loc)
}

#[async_trait]
impl SearchPlugin for Xiaozhang {
    fn name(&self) -> &'static str { "xiaozhang" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = format!("https://xzys.fun/search.html?keyword={}", q(keyword));
        let body = client.get_text(&url, Some(browser_headers(Some("https://xzys.fun")))).await?;
        let items = xiaozhang_parse_list(&body);
        let mut futs = Vec::new();
        for (id, detail, title, content) in items {
            let c = client.clone();
            futs.push(async move {
                // 第一步禁跟随重定向取 Location
                let real = xiaozhang_location(&c, &detail).await;
                let target = match real {
                    Some(loc) if !loc.is_empty() => {
                        if loc.starts_with("http") { loc } else { format!("https://xzys.fun{}", loc) }
                    }
                    _ => detail.clone(),
                };
                let dbody = c.get_text(&target, Some(browser_headers(Some(&detail)))).await.ok();
                (id, title, content, dbody)
            });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (id, title, content, dbody) in details {
            let links = match dbody { Some(b) => xiaozhang_parse_detail(&b), None => Vec::new() };
            if links.is_empty() { continue; }
            let mut r = result("xiaozhang", &id, &title);
            r.content = content;
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// pianku：两步 HTML，https://btnull.pro/search/-------------.html?wd=
// 列表 .sr_lists dl → 详情 #donLink .down-list2 .down-list3 a[href]
// ext.title_en 可覆盖关键词；网盘 10 类 + magnet + ed2k
// ============================================================

pub struct Pianku;

fn pianku_parse_list(body: &str) -> Vec<(String, String, String, String, Vec<String>)> {
    // (id, detail_url, title, content, tags)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    static ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/movie/(\d+)\.html").unwrap());
    static RT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"地区：([^　]*?)　+类型：(.*)").unwrap());
    for item in sel(&doc, ".sr_lists dl") {
        let a = match sel(&item, "dt a").into_iter().next() { Some(e) => e, None => continue };
        let href = a.value().attr("href").unwrap_or("").to_string();
        let id = match ID_RE.captures(&href).and_then(|c| c.get(1)) {
            Some(m) => m.as_str().to_string(), None => continue,
        };
        let title = sel(&item, "dd p:first-child strong a").into_iter().next()
            .map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        if title.is_empty() { continue; }
        let status = sel(&item, "dd p:first-child span.ss1").into_iter().next()
            .map(|e| e.text().collect::<String>().trim().to_string()).unwrap_or_default();
        let mut actors = String::new();
        let mut desc = String::new();
        let mut alt = String::new();
        let mut region = String::new();
        let mut types = String::new();
        for p in sel(&item, "dd p") {
            let text = p.text().collect::<String>().trim().to_string();
            if let Some(rest) = text.strip_prefix("又名：") { alt = rest.to_string(); }
            else if text.contains("地区：") && text.contains("类型：") {
                if let Some(c) = RT_RE.captures(&text) {
                    region = c.get(1).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                    types = c.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                }
            } else if let Some(rest) = text.strip_prefix("主演：") { actors = rest.to_string(); }
            else if let Some(rest) = text.strip_prefix("简介：") { desc = rest.to_string(); }
            else if !text.contains("名称：") && !text.contains("地区：") && !text.contains("主演：") && desc.is_empty() && text.len() > 10 {
                desc = text;
            }
        }
        let mut tags = Vec::new();
        if !region.is_empty() { tags.push(region); }
        for t in types.split(',') { let t = t.trim(); if !t.is_empty() { tags.push(t.to_string()); } }
        if !status.is_empty() { tags.push(status); }
        let mut content = desc;
        if !actors.is_empty() {
            content = if content.is_empty() { format!("主演：{}", actors) } else { format!("主演：{}\n{}", actors, content) };
        }
        if !alt.is_empty() {
            content = if content.is_empty() { format!("又名：{}", alt) } else { format!("又名：{}\n{}", alt, content) };
        }
        let detail = if href.starts_with("http") { href } else { format!("https://btnull.pro{}", href) };
        out.push((id, detail, title, content, tags));
    }
    out
}

fn pianku_parse_detail(body: &str) -> Vec<Link> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for box_el in sel(&doc, "#donLink .down-list2") {
        let a = match sel(&box_el, ".down-list3 a").into_iter().next() { Some(e) => e, None => continue };
        let href = a.value().attr("href").unwrap_or("").to_string();
        let title = a.text().collect::<String>().trim().to_string();
        if href.is_empty() || title.is_empty() { continue; }
        let ty = determine_type(&href);
        if ty == "others" && !href.starts_with("magnet:") && !href.starts_with("ed2k:") { continue; }
        if out.iter().any(|l: &Link| l.url == href) { continue; }
        let mut pass = url_password(&href);
        if pass.is_empty() { pass = password_near(&title); }
        out.push(Link { r#type: ty.to_string(), url: href, password: pass, datetime: None, work_title: None });
    }
    out
}

#[async_trait]
impl SearchPlugin for Pianku {
    fn name(&self) -> &'static str { "pianku" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, ext: &ExtMap) -> Result<Vec<SearchResult>> {
        // ext.title_en 可覆盖关键词
        let kw = ext.get("title_en").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(keyword);
        let url = format!("https://btnull.pro/search/-------------.html?wd={}", q(kw));
        let body = client.get_text(&url, Some(browser_headers(Some("https://btnull.pro/")))).await?;
        let items = pianku_parse_list(&body);
        let mut futs = Vec::new();
        for (id, detail, title, content, tags) in items {
            let c = client.clone();
            futs.push(async move {
                let dbody = c.get_text(&detail, Some(browser_headers(Some("https://btnull.pro/")))).await.ok();
                (id, title, content, tags, dbody)
            });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (id, title, content, tags, dbody) in details {
            let links = match dbody { Some(b) => pianku_parse_detail(&b), None => Vec::new() };
            if links.is_empty() { continue; }
            let mut r = result("pianku", &id, &title);
            r.content = content;
            r.tags = tags;
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}
