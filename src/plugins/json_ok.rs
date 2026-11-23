//! JSON / 特殊结构插件（已验证可用）：nyaa(BT单步) / pansearch(Next.js JSON) / qiwei(suggest+详情)
//! 解析逻辑同步化。

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{clean_html, determine_type, first_text, password_near, sel, url_password};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::HeaderMap;
use scraper::Html;
use serde::Deserialize;

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
// nyaa：单步 BT 搜索，https://nyaa.si/?f=0&c=0_0&q=
// 表格 table.torrent-list tbody tr → magnet + 元信息
// ext.title_en 可覆盖关键词；跳过 Service 过滤
// ============================================================

pub struct Nyaa;
static NYAA_VIEW_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/view/(\d+)").unwrap());

fn nyaa_parse(body: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    let tbody = match sel(&doc, "table.torrent-list tbody").into_iter().next() { Some(t) => t, None => return out };
    for tr in sel(&tbody, "tr") {
        let tds = sel(&tr, "td");
        if tds.len() < 6 { continue; }
        // 标题链接在 td[colspan='2'] a
        let title_a = sel(&tr, "td[colspan='2'] a").into_iter().last()
            .or_else(|| sel(&tr, "a[href*='/view/']").into_iter().next());
        let (title, href) = match title_a {
            Some(a) => (
                { let t = a.text().collect::<String>().trim().to_string();
                  if t.is_empty() { a.value().attr("title").unwrap_or("").to_string() } else { t } },
                a.value().attr("href").unwrap_or("").to_string(),
            ),
            None => continue,
        };
        let id = NYAA_VIEW_RE.captures(&href).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());
        let id = match id { Some(x) => x, None => continue };
        // magnet
        let magnet = sel(&tr, "a[href^='magnet:']").into_iter().next()
            .and_then(|a| a.value().attr("href").map(|s| s.to_string()));
        let magnet = match magnet { Some(m) => m, None => continue };
        let category = sel(&tr, "td:nth-child(1) a").into_iter().next()
            .and_then(|a| a.value().attr("title").map(|s| s.to_string())).unwrap_or_default();
        // 文本单元格
        let cells: Vec<String> = tds.iter().map(|td| td.text().collect::<String>().trim().to_string()).collect();
        let size = cells.get(1).cloned().unwrap_or_default();
        let n = cells.len();
        let seeders = cells.get(n - 3).cloned().unwrap_or_default();
        let leechers = cells.get(n - 2).cloned().unwrap_or_default();
        let completed = cells.get(n - 1).cloned().unwrap_or_default();
        let ts = sel(&tr, "td[data-timestamp]").into_iter().next()
            .and_then(|td| td.value().attr("data-timestamp").and_then(|s| s.parse::<i64>().ok()));
        let mut r = result("nyaa", &id, &title);
        if let Some(t) = ts { r.datetime = chrono::DateTime::from_timestamp(t, 0).unwrap_or_else(Utc::now); }
        r.content = format!("分类: {} | 大小: {} | 做种: {} | 下载: {} | 完成: {}", category, size, seeders, leechers, completed);
        r.tags = vec![category, format!("做种:{}", seeders), format!("下载:{}", leechers), format!("完成:{}", completed)];
        r.links.push(Link { r#type: "magnet".into(), url: magnet, password: String::new(), datetime: None, work_title: None });
        out.push(r);
    }
    out
}

#[async_trait]
impl SearchPlugin for Nyaa {
    fn name(&self) -> &'static str { "nyaa" }
    fn priority(&self) -> i32 { 3 }
    fn skip_service_filter(&self) -> bool { true }
    async fn search(&self, client: &HttpClient, keyword: &str, ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let kw = ext.get("title_en").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(keyword);
        let url = format!("https://nyaa.si/?f=0&c=0_0&q={}", q(kw));
        let body = client.get_text(&url, Some(browser_headers(Some("https://nyaa.si")))).await?;
        Ok(nyaa_parse(&body))
    }
}

// ============================================================
// pansearch：Next.js buildId → https://www.pansearch.me/_next/data/<buildId>/search.json?keyword=&offset=
// JSON: pageProps.data.{total,data[]}；item {id,content,pan,image,time}
// content 是 HTML 片段，含 href="链接" 与 ?pwd=密码
// ============================================================

pub struct Pansearch;
static BUILD_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""buildId":"([^"]+)""#).unwrap());
static HREF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"href="([^"]+)""#).unwrap());
static PWD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r##"\?pwd=([^"#]+)"##).unwrap());
static NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"名称：([^\n<]+)").unwrap());

#[derive(Debug, Deserialize, Default)]
struct PansearchResp {
    #[serde(default, rename = "pageProps")] page_props: PansearchPageProps,
}
#[derive(Debug, Deserialize, Default)]
struct PansearchPageProps {
    #[serde(default)] data: PansearchData,
}
#[derive(Debug, Deserialize, Default)]
struct PansearchData {
    #[serde(default)] total: i64,
    #[serde(default)] data: Vec<PansearchItem>,
}
#[derive(Debug, Deserialize, Default)]
struct PansearchItem {
    #[serde(default)] id: i64,
    #[serde(default)] content: String,
    #[serde(default)] pan: String,
    #[serde(default)] image: String,
    #[serde(default)] time: String,
}

fn pansearch_clean(s: &str) -> String {
    let mut v = s.replace("<span class='highlight-keyword'>", "").replace("</span>", "")
        .replace("<a class=\"resource-link\" target=\"_blank\" href=\"", "")
        .replace("</a>", "").replace("<br>", "\n").replace("<p>", "").replace("</p>", "\n");
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").unwrap());
    v = TAG.replace_all(&v, "").to_string();
    clean_html(&v)
}

fn pansearch_items(body: &str, keyword: &str) -> (Vec<PansearchItem>, i64) {
    let resp: PansearchResp = serde_json::from_str(body).unwrap_or_default();
    let total = resp.page_props.data.total;
    let mut items = resp.page_props.data.data;
    // 关键词过滤（content）
    items.retain(|it| crate::util::text::matches_keywords("", &it.content, keyword));
    (items, total)
}

fn pansearch_convert(items: Vec<PansearchItem>) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for it in items {
        let url = match HREF_RE.captures(&it.content).and_then(|c| c.get(1)) {
            Some(m) => m.as_str().to_string(), None => continue,
        };
        let pass = PWD_RE.captures(&it.content).and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string()).unwrap_or_else(|| url_password(&url));
        let title = NAME_RE.captures(&it.content).and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty())
            .unwrap_or_else(|| pansearch_clean(&it.content).lines().next().unwrap_or("").to_string());
        let ty = if it.pan == "aliyundrive" { "aliyun" } else { determine_type(&it.pan) };
        let ty = if ty == "others" { determine_type(&url) } else { ty };
        let mut r = result("pansearch", &it.id.to_string(), &title);
        r.content = pansearch_clean(&it.content);
        r.datetime = crate::plugins::helpers::parse_time(&it.time);
        if !it.image.is_empty() { r.images.push(it.image); }
        r.links.push(Link { r#type: ty.to_string(), url, password: pass, datetime: None, work_title: None });
        out.push(r);
    }
    out
}

#[async_trait]
impl SearchPlugin for Pansearch {
    fn name(&self) -> &'static str { "pansearch" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        // 获取 buildId
        let home = client.get_text("https://www.pansearch.me/search", Some(browser_headers(None))).await?;
        let build_id = BUILD_ID_RE.captures(&home).and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .or_else(|| {
                // __NEXT_DATA__ 兜底
                serde_json::from_str::<serde_json::Value>(&home).ok()
                    .and_then(|v| v.get("buildId").and_then(|x| x.as_str()).map(|s| s.to_string()))
            });
        let build_id = match build_id { Some(b) => b, None => return Ok(Vec::new()) };
        let mut headers = browser_headers(Some("https://www.pansearch.me/"));
        headers.insert("accept", reqwest::header::HeaderValue::from_static("application/json, text/plain, */*"));
        headers.insert("cache-control", reqwest::header::HeaderValue::from_static("no-cache"));
        // 第一页
        let url = format!("https://www.pansearch.me/_next/data/{}/search.json?keyword={}&offset=0", build_id, q(keyword));
        let body = match client.get_text(&url, Some(headers.clone())).await { Ok(b) => b, Err(_) => return Ok(Vec::new()) };
        let (items, total) = pansearch_items(&body, keyword);
        let mut all = items;
        // 后续页（最多 9 页）
        let pages = (((total - 10).max(0) as f64) / 10.0).ceil() as usize;
        let pages = pages.min(9);
        if pages > 0 {
            let c = client.clone();
            let kw = q(keyword);
            let mut futs = Vec::new();
            for i in 0..pages {
                let u = format!("https://www.pansearch.me/_next/data/{}/search.json?keyword={}&offset={}", build_id, kw, 10 + i * 10);
                let h = headers.clone();
                let c2 = c.clone();
                futs.push(async move { c2.get_text(&u, Some(h)).await });
            }
            for b in futures::future::join_all(futs).await.into_iter().flatten() {
                let (it, _) = pansearch_items(&b, keyword);
                all.extend(it);
            }
        }
        let mut seen = std::collections::HashSet::new();
        all.retain(|it| seen.insert(it.id));
        Ok(pansearch_convert(all))
    }
}

// ============================================================
// qiwei：域名池 failover，suggest JSON → 详情页
// GET <host>/index.php/ajax/suggest?mid=1&limit=100&wd=
// suggest {code,list:[{id,name,pic,en}]}；详情 <host>/mv/<id>.html
// ============================================================

pub struct Qiwei;
const QIWEI_HOSTS: &[&str] = &[
    "https://www.qnmp4.com", "https://www.qwfilm.com", "https://www.qwmkv.com",
    "https://www.qwnull.com", "https://www.qn63.com",
];
static QIWEI_UPDATE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2})").unwrap());

#[derive(Debug, Deserialize, Default)]
struct QiweiSuggest {
    #[serde(default)] code: i32,
    #[serde(default)] list: Vec<QiweiItem>,
}
#[derive(Debug, Deserialize, Default)]
struct QiweiItem {
    #[serde(default)] id: i64,
    #[serde(default)] name: String,
    #[serde(default)] pic: String,
}

fn qiwei_detail_links(body: &str) -> Vec<(String, String)> {
    // 返回 (url, password)
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    let mut push = |url: &str, ctx: &str| {
        if url.is_empty() { return; }
        let ty = determine_type(url);
        if ty == "others" && !url.starts_with("magnet:") && !url.starts_with("ed2k:") { return; }
        if out.iter().any(|x: &(String, String)| x.0 == url) { return; }
        let pass = { let p = url_password(url); if p.is_empty() { password_near(ctx) } else { p } };
        out.push((url.to_string(), pass));
    };
    for box_el in sel(&doc, ".down-list .content li.down-list2") {
        for a in sel(&box_el, "a.folder, span a[href]") {
            let href = a.value().attr("href").unwrap_or("");
            let ctx = format!("{} {}", a.text().collect::<String>(), href);
            push(href, &ctx);
        }
    }
    // magnet 从原始 HTML
    static MAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"magnet:\?[^\s"'<>]+"#).unwrap());
    for m in MAG_RE.find_iter(body) { push(m.as_str(), ""); }
    out
}

#[async_trait]
impl SearchPlugin for Qiwei {
    fn name(&self) -> &'static str { "qiwei" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        // 找可用 host（suggest 返回有效 JSON）
        let mut chosen: Option<&str> = None;
        let mut suggest: Option<QiweiSuggest> = None;
        for host in QIWEI_HOSTS {
            let url = format!("{}/index.php/ajax/suggest?mid=1&limit=100&wd={}", host, q(keyword));
            let mut h = browser_headers(Some(&format!("{}/", host)));
            h.insert("accept", reqwest::header::HeaderValue::from_static("application/json, text/plain, */*"));
            if let Ok(body) = client.get_text(&url, Some(h)).await {
                if let Ok(s) = serde_json::from_str::<QiweiSuggest>(&body) {
                    if s.code == 1 || !s.list.is_empty() {
                        chosen = Some(host); suggest = Some(s); break;
                    }
                }
            }
        }
        let host = match chosen { Some(h) => h, None => return Ok(Vec::new()) };
        let suggest = suggest.unwrap();
        // 并发抓详情
        let mut futs = Vec::new();
        for it in suggest.list {
            let url = format!("{}/mv/{}.html", host, it.id);
            let c = client.clone();
            let h = browser_headers(Some(&format!("{}/", host)));
            futs.push(async move { let b = c.get_text(&url, Some(h)).await.ok(); (it, b) });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (it, dbody) in details {
            let dbody = match dbody { Some(b) => b, None => continue };
            let links = qiwei_detail_links(&dbody);
            if links.is_empty() { continue; }
            let doc = Html::parse_document(&dbody);
            let mut title = first_text(&doc, ".main-ui-meta h1");
            if title.is_empty() { title = it.name.clone(); }
            static YEAR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*\(\d{4}\)\s*$").unwrap());
            title = YEAR_RE.replace(&title, "").to_string();
            let mut r = result("qiwei", &it.id.to_string(), &title);
            r.content = first_text(&doc, ".main-ui-meta .otherbox");
            if r.content.is_empty() { r.content = first_text(&doc, ".movie-introduce .sqjj_a"); }
            if !it.pic.is_empty() { r.images.push(it.pic); }
            let full = doc.root_element().text().collect::<String>();
            if let Some(c) = QIWEI_UPDATE_RE.captures(&full) {
                r.datetime = crate::plugins::helpers::parse_time(c.get(1).map(|m| m.as_str()).unwrap_or(""));
            }
            r.links = links.into_iter().map(|(url, pass)| Link {
                r#type: determine_type(&url).to_string(), url, password: pass, datetime: None, work_title: Some(title.clone()),
            }).collect();
            out.push(r);
        }
        Ok(out)
    }
}
