//! 新版本（pansou-main）独有的插件适配，旧版本 pansou-master 中没有。
//! 本批全部经过实测连通性 + 结构确认，采用 port 自 Go 实现的解析逻辑。
//!
//! 已实现：
//!   - ikantv    : JSON 一步，api.naspt.vip/api/open/pansou/search
//!   - quarkres  : JSON 一步，squark.cc.cd/api/search（仅夸克）
//!   - aipan     : JSON 两步，www.aipan.me/api/movies/search -> detail
//!   - haitunsou : HTML 内嵌 JSON（const list = JSON.parse('...')），与 yunsou 同类
//!   - xiaoyu    : HTML 一步，xykmovie.com/s/{page}/{kw}.html，base64 data-url

use crate::model::{Link, SearchResult};
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::helpers::{clean_html, determine_type, password_near, url_password};
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::HeaderValue;
use serde_json::Value;

fn q(s: &str) -> String { crate::util::text::url_encode(s) }

fn base_result(name: &str, uid: &str, title: &str) -> SearchResult {
    SearchResult {
        unique_id: format!("{}-{}", name, uid),
        message_id: format!("{}-{}", name, uid),
        title: title.to_string(),
        datetime: Utc::now(),
        channel: String::new(),
        ..Default::default()
    }
}

/// 把 JSON 里的 type 字段规范化到内部网盘类型
fn norm_type(raw: &str, url: &str) -> String {
    let t = raw.trim().to_lowercase();
    let mapped = match t.as_str() {
        "alipan" | "aliyun" | "aliyundrive" | "ali" => "aliyun",
        "189cloud" | "tianyi" => "tianyi",
        "caiyun" | "mobile" | "139" => "mobile",
        "guangya" | "guangya_pan" => "guangya",
        "123pan" | "123" => "123",
        "baidu" | "quark" | "uc" | "xunlei" | "115" | "pikpak" | "magnet" | "ed2k" => t.as_str(),
        "" | "unknown" | "others" => determine_type(url),
        _ => determine_type(url),
    };
    if mapped == "others" { determine_type(url).to_string() } else { mapped.to_string() }
}

/// 校验链接是否为可用的网盘/磁力地址
fn valid_url(url: &str) -> bool {
    let u = url.trim();
    let l = u.to_lowercase();
    if l.starts_with("magnet:") || l.starts_with("ed2k://") { return true; }
    l.starts_with("http://") || l.starts_with("https://")
}

// ============================================================
// ikantv：JSON 一步
// GET https://api.naspt.vip/api/open/pansou/search?kw=&limit=50
// {code:0, data:[{message_id, unique_id, datetime, title, content, tags[], images[], links:[{type,url,password,datetime,work_title}]}]}
// ============================================================

/// ikantv 插件
pub struct IkanTv;

#[async_trait]
impl SearchPlugin for IkanTv {
    fn name(&self) -> &'static str { "ikantv" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let kw = ext.get("title_en").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(keyword);
        let mut url = format!("https://api.naspt.vip/api/open/pansou/search?kw={}&limit=50", q(kw));
        if let Some(ten) = ext.get("title_en").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            url.push_str(&format!("&title_en={}", q(ten)));
        }
        let mut h = browser_headers(Some("https://api.naspt.vip/"));
        h.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
        let body = client.get_text(&url, Some(h)).await?;
        let v: Value = match serde_json::from_str(&body) { Ok(v) => v, Err(_) => return Ok(vec![]) };
        if v.get("code").and_then(|x| x.as_i64()) != Some(0) { return Ok(vec![]); }
        let mut out = Vec::new();
        if let Some(items) = v.get("data").and_then(|x| x.as_array()) {
            for item in items {
                let mut links = Vec::new();
                if let Some(ls) = item.get("links").and_then(|x| x.as_array()) {
                    for l in ls {
                        let raw = l.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
                        if !valid_url(raw) { continue; }
                        let ty_raw = l.get("type").and_then(|x| x.as_str()).unwrap_or("");
                        let ty = norm_type(ty_raw, raw);
                        // 过滤非网盘类型
                        if matches!(ty.as_str(), "others" | "") { continue; }
                        let pwd = l.get("password").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                        let wt = l.get("work_title").and_then(|x| x.as_str()).map(|s| s.to_string());
                        links.push(Link { r#type: ty, url: raw.to_string(), password: pwd, datetime: None, work_title: wt });
                    }
                }
                if links.is_empty() { continue; }
                let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let content = item.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let msg_id = item.get("message_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let uid = item.get("unique_id").and_then(|x| x.as_str())
                    .map(|s| s.strip_prefix("ikantv-").unwrap_or(s).to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| msg_id.clone());
                let mut r = base_result("ikantv", &uid, &title);
                r.message_id = msg_id;
                r.content = content;
                r.links = links;
                if let Some(tags) = item.get("tags").and_then(|x| x.as_array()) {
                    r.tags = tags.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect();
                }
                if let Some(imgs) = item.get("images").and_then(|x| x.as_array()) {
                    r.images = imgs.iter().filter_map(|t| t.as_str().map(|s| s.to_string())).collect();
                }
                if let Some(dt) = item.get("datetime").and_then(|x| x.as_str()) {
                    r.datetime = crate::plugins::helpers::parse_time(dt);
                }
                out.push(r);
            }
        }
        Ok(out)
    }
}

// ============================================================
// quarkres：JSON 一步，仅夸克
// GET https://squark.cc.cd/api/search?kw=
// {code:200, data:[{unique_id,title,content,datetime,links:[{type,url,password}]}]}
// ============================================================

pub struct QuarkRes;

#[async_trait]
impl SearchPlugin for QuarkRes {
    fn name(&self) -> &'static str { "quarkres" }
    fn priority(&self) -> i32 { 2 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = format!("https://squark.cc.cd/api/search?kw={}", q(keyword));
        let mut h = browser_headers(Some("https://squark.cc.cd/"));
        h.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
        let body = client.get_text(&url, Some(h)).await?;
        let v: Value = match serde_json::from_str(&body) { Ok(v) => v, Err(_) => return Ok(vec![]) };
        if v.get("code").and_then(|x| x.as_i64()) != Some(200) { return Ok(vec![]); }
        let mut out = Vec::new();
        if let Some(items) = v.get("data").and_then(|x| x.as_array()) {
            for item in items {
                let mut links = Vec::new();
                if let Some(ls) = item.get("links").and_then(|x| x.as_array()) {
                    for l in ls {
                        let raw = l.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
                        if raw.is_empty() || !raw.contains("pan.quark.cn") { continue; }
                        let pwd = l.get("password").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                        links.push(Link { r#type: "quark".into(), url: raw.to_string(), password: pwd, datetime: None, work_title: None });
                    }
                }
                if links.is_empty() { continue; }
                let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let content = item.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let uid = item.get("unique_id").and_then(|x| x.as_str())
                    .map(|s| s.strip_prefix("quarkres-").unwrap_or(s).to_string())
                    .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string());
                let mut r = base_result("quarkres", &uid, &title);
                r.content = content;
                r.links = links;
                if let Some(dt) = item.get("datetime").and_then(|x| x.as_str()) {
                    r.datetime = crate::plugins::helpers::parse_time(dt);
                }
                out.push(r);
            }
        }
        Ok(out)
    }
}

// ============================================================
// aipan：JSON 两步
// GET /api/movies/search?q=   -> {movies:[{id,title,cover,year,genres,...}]}
// GET /api/movies/detail/{id} -> {title,panLinks:[{kind,url,title}],resources:[{kind,url,name}],...}
// ============================================================

pub struct Aipan;

fn aipan_link_type(kind: &str, url: &str) -> Option<String> {
    let k = kind.trim().to_lowercase();
    let direct = match k.as_str() {
        "magnet" if url.to_lowercase().starts_with("magnet:") => Some("magnet"),
        "ed2k" if url.to_lowercase().starts_with("ed2k://") => Some("ed2k"),
        "alipan" | "aliyun" | "aliyundrive" | "ali" => Some("aliyun"),
        "guangya" | "guangya_pan" => Some("guangya"),
        "quark" => Some("quark"), "baidu" => Some("baidu"), "uc" => Some("uc"),
        "xunlei" => Some("xunlei"), "tianyi" | "189cloud" => Some("tianyi"),
        "115" => Some("115"), "123" | "123pan" => Some("123"),
        "pikpak" => Some("pikpak"), "mobile" | "139" => Some("mobile"),
        _ => None,
    };
    if direct.is_some() { return direct.map(|s| s.to_string()); }
    let t = determine_type(url);
    if t == "others" { None } else { Some(t.to_string()) }
}

fn aipan_password(url: &str, label: &str) -> String {
    let p = url_password(url);
    if !p.is_empty() { return p; }
    password_near(label)
}

#[async_trait]
impl SearchPlugin for Aipan {
    fn name(&self) -> &'static str { "aipan" }
    fn priority(&self) -> i32 { 2 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let mut h = browser_headers(Some("https://www.aipan.me/"));
        h.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
        h.insert("origin", HeaderValue::from_static("https://www.aipan.me"));
        let search_url = format!("https://www.aipan.me/api/movies/search?q={}", q(keyword));
        let body = client.get_text(&search_url, Some(h.clone())).await?;
        let v: Value = match serde_json::from_str(&body) { Ok(v) => v, Err(_) => return Ok(vec![]) };
        let movies = match v.get("movies").and_then(|x| x.as_array()) { Some(m) => m, None => return Ok(vec![]) };
        // 取前 12 个
        let mut futs = Vec::new();
        for item in movies.iter().take(12) {
            let id = item.get("id").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let cover = item.get("cover").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if id.is_empty() || title.is_empty() { continue; }
            let c = client.clone();
            let hd = h.clone();
            let id2 = id.clone();
            futs.push(async move {
                let u = format!("https://www.aipan.me/api/movies/detail/{}", q(&id2));
                let r = c.get_text(&u, Some(hd)).await.ok();
                (id2, title, cover, r)
            });
        }
        let details = futures::future::join_all(futs).await;
        let mut out = Vec::new();
        for (id, search_title, cover, detail_res) in details {
            let dbody = match detail_res { Some(b) => b, None => continue };
            let d: Value = match serde_json::from_str(&dbody) { Ok(v) => v, Err(_) => continue };
            let title = {
                let t = d.get("title").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if t.is_empty() { search_title.clone() } else { t }
            };
            let mut links = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut add = |kind: &str, raw: &str, label: &str| {
                let raw = raw.trim().trim_matches(|c: char| matches!(c, '"' | '\'' | '<' | '>' | ')' | '.' | ';')).replace("&amp;", "&");
                if !valid_url(&raw) { return; }
                let ty = match aipan_link_type(kind, &raw) { Some(t) => t, None => return };
                let pwd = aipan_password(&raw, label);
                let key = format!("{}\0{}\0{}", ty, raw, pwd);
                if !seen.insert(key) { return; }
                links.push(Link { r#type: ty, url: raw, password: pwd, datetime: None, work_title: Some(title.clone()) });
            };
            if let Some(pl) = d.get("panLinks").and_then(|x| x.as_array()) {
                for l in pl {
                    add(l.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
                        l.get("url").and_then(|x| x.as_str()).unwrap_or(""),
                        l.get("title").and_then(|x| x.as_str()).unwrap_or(""));
                }
            }
            if let Some(rs) = d.get("resources").and_then(|x| x.as_array()) {
                for l in rs {
                    add(l.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
                        l.get("url").and_then(|x| x.as_str()).unwrap_or(""),
                        l.get("name").and_then(|x| x.as_str()).unwrap_or(""));
                }
            }
            if links.is_empty() { continue; }
            let mut r = base_result("aipan", &id, &title);
            // content 组装
            let mut parts = Vec::new();
            if let Some(aka) = d.get("aka").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) { parts.push(format!("别名：{}", aka)); }
            if let Some(y) = d.get("year").and_then(|x| x.as_i64()).filter(|y| *y > 0) { parts.push(format!("年份：{}", y)); }
            if let Some(syn) = d.get("synopsis").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) { parts.push(syn.to_string()); }
            r.content = clean_html(&parts.join(" | "));
            let mut tags = vec!["爱盼".to_string()];
            if let Some(gs) = d.get("genres").and_then(|x| x.as_array()) {
                tags.extend(gs.iter().filter_map(|g| g.as_str().map(|s| s.to_string())));
            }
            r.tags = tags;
            let cov = d.get("posterUrl").and_then(|x| x.as_str()).filter(|s| !s.is_empty())
                .unwrap_or(&cover);
            if !cov.is_empty() { r.images.push(cov.to_string()); }
            r.links = links;
            out.push(r);
        }
        Ok(out)
    }
}

// ============================================================
// haitunsou：HTML 内嵌 JSON
// GET https://www.haitunsou.com/s/{kw}.html
// 页面含 `const list = JSON.parse('[{...}]');`
// item {id,title,is_type,code,url,name,times,category}
// ============================================================

static HTS_MARKERS: &[&str] = &["const list = JSON.parse('", "let listItems = JSON.parse('"];

fn hts_extract(body: &str) -> String {
    for marker in HTS_MARKERS {
        if let Some(pos) = body.find(marker) {
            let rest = &body[pos + marker.len()..];
            // JSON.parse 单引号字符串，处理 \/ 与 \'
            if let Some(end) = find_unescaped_quote(rest, '\'') {
                return rest[..end].replace("\\/", "/").replace("\\'", "'");
            }
        }
    }
    String::new()
}

/// 找到未被反斜杠转义的引号位置
fn find_unescaped_quote(s: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped { escaped = false; continue; }
        if c == '\\' { escaped = true; continue; }
        if c == quote { return Some(i); }
    }
    None
}

pub struct Haitunsou;

#[async_trait]
impl SearchPlugin for Haitunsou {
    fn name(&self) -> &'static str { "haitunsou" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let url = format!("https://www.haitunsou.com/s/{}.html", q(keyword));
        let body = client.get_text(&url, Some(browser_headers(Some("https://www.haitunsou.com/")))).await?;
        let json_str = hts_extract(&body);
        if json_str.is_empty() { return Ok(vec![]); }
        let arr: Value = match serde_json::from_str(&json_str) { Ok(v) => v, Err(_) => return Ok(vec![]) };
        let mut out = Vec::new();
        if let Some(items) = arr.as_array() {
            for item in items {
                let mut raw = item.get("url").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if raw.len() < 8 { continue; }
                if raw.contains("pan.qualk.cn") { raw = raw.replace("pan.qualk.cn", "pan.quark.cn"); }
                let is_type = item.get("is_type").and_then(|x| x.as_i64()).unwrap_or(-1);
                let ty = match is_type {
                    0 => "quark", 1 => "aliyun", 2 => "baidu", 3 => "uc", 4 => "xunlei",
                    _ => determine_type(&raw),
                };
                let mut pwd = item.get("code").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if pwd.is_empty() { pwd = url_password(&raw); }
                let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let id = item.get("id").and_then(|x| x.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| item.get("id").and_then(|x| x.as_i64()).map(|n| n.to_string()).unwrap_or_default());
                let times = item.get("times").and_then(|x| x.as_str()).unwrap_or("");
                let mut r = base_result("haitunsou", &id, &title);
                r.content = name;
                r.datetime = crate::plugins::helpers::parse_time(times);
                r.links.push(Link { r#type: ty.to_string(), url: raw, password: pwd, datetime: None, work_title: None });
                out.push(r);
            }
        }
        Ok(out)
    }
}

// ============================================================
// xiaoyu：HTML 一步
// GET https://xykmovie.com/s/1/{kw} （注意路径不带 .html，否则会被并入关键词）
// .item 容器： a.open[data-url(base64)][data-code][data-id] 标题, a.copy[data-code] 描述
// ============================================================

use scraper::Html;

fn xy_decode_data_url(value: &str) -> String {
    use base64::Engine;
    let v = value.trim();
    if v.is_empty() { return String::new(); }
    let decoded = base64::engine::general_purpose::STANDARD.decode(v)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(v));
    let mut result = match decoded {
        Ok(d) => String::from_utf8_lossy(&d).trim().to_string(),
        Err(_) => return String::new(),
    };
    if !result.is_empty() && !result.contains("://") {
        result = format!("https://{}", result.trim_start_matches('/'));
    }
    result
}

fn xy_normalize_link(raw: &str) -> Option<(String, String)> {
    let raw = clean_html(raw).trim().to_string();
    let u = url::Url::parse(&raw).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" { return None; }
    let host = u.host_str()?.to_lowercase();
    let ty = match host.as_str() {
        "pan.quark.cn" => "quark", "pan.baidu.com" => "baidu", "pan.xunlei.com" => "xunlei",
        "drive.uc.cn" => "uc", "www.alipan.com" | "alipan.com" | "www.aliyundrive.com" | "aliyundrive.com" => "aliyun",
        "cloud.189.cn" => "tianyi", "115.com" | "115cdn.com" => "115",
        "www.123pan.com" | "123pan.com" | "www.123684.com" | "123684.com" => "123",
        "caiyun.139.com" => "mobile", "mypikpak.com" => "pikpak",
        _ => return None,
    };
    Some((ty.to_string(), u.to_string()))
}

fn xy_clean_pwd(value: &str) -> String {
    value.trim().trim_matches(|c: char| matches!(c, '#' | '，' | ',' | '。' | '.' | ';' | '；' | ':' | '：' | '(' | ')' | '（' | '）' | '[' | ']' | '【' | '】')).to_string()
}

fn xy_password(url: &str, copy_text: &str, open_code: &str) -> String {
    if let Ok(u) = url::Url::parse(url) {
        for k in ["pwd", "password", "code"] {
            let v = xy_clean_pwd(u.query_pairs().find(|(key, _)| key == k).map(|(_, val)| val.as_ref().to_string()).unwrap_or_default().as_str());
            if !v.is_empty() { return v; }
        }
    }
    let p = xy_clean_pwd(open_code);
    if !p.is_empty() { return p; }
    for marker in ["提取码:", "提取码：", "访问码:", "访问码：", "密码:", "密码："] {
        if let Some(i) = copy_text.find(marker) {
            let rest = &copy_text[i + marker.len()..];
            if let Some(tok) = rest.split_whitespace().next() {
                let v = xy_clean_pwd(tok);
                if !v.is_empty() { return v; }
            }
        }
    }
    String::new()
}

fn xy_parse(body: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let mut out = Vec::new();
    for item in crate::plugins::helpers::sel(&doc, ".search-list .item, .item") {
        let open = crate::plugins::helpers::sel(&item, ".name a.open").into_iter().next()
            .or_else(|| crate::plugins::helpers::sel(&item, "a.open").into_iter().next());
        let open = match open { Some(o) => o, None => continue };
        let title = open.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
        let title = title.trim().to_string();
        if title.is_empty() { continue; }
        let data_url = open.value().attr("data-url").unwrap_or("");
        let open_code = open.value().attr("data-code").unwrap_or("");
        let data_id = open.value().attr("data-id").unwrap_or("");
        let mut raw_url = xy_decode_data_url(data_url);
        let copy_text = clean_html(crate::plugins::helpers::sel(&item, "a.copy[data-code]").into_iter().next()
            .and_then(|a| a.value().attr("data-code")).unwrap_or(""));
        if raw_url.is_empty() {
            static URLRE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"https?://[^\s"'<>]+"#).unwrap());
            raw_url = URLRE.find(&copy_text).map(|m| m.as_str().to_string()).unwrap_or_default();
        }
        let (ty, url) = match xy_normalize_link(&raw_url) { Some(x) => x, None => continue };
        let pwd = xy_password(&url, &copy_text, open_code);
        let id = if data_id.trim().is_empty() {
            use sha2::{Sha256, Digest};
            format!("{:x}", Sha256::digest(format!("{}\0{}", title, url).as_bytes()))[..16].to_string()
        } else { data_id.trim().to_string() };
        let atips = crate::plugins::helpers::sel(&item, ".atips").into_iter()
            .map(|e| e.text().collect::<String>().trim().to_string()).collect::<Vec<_>>();
        let datetime = atips.first().map(|s| crate::plugins::helpers::parse_time(s)).unwrap_or_else(Utc::now);
        let content = atips.get(1).map(|s| s.trim_start_matches("【内容】：").to_string()).unwrap_or_default();
        let mut r = base_result("xiaoyu", &id, &title);
        r.content = content;
        r.datetime = datetime;
        r.links.push(Link { r#type: ty, url, password: pwd, datetime: None, work_title: Some(title.clone()) });
        out.push(r);
    }
    out
}

pub struct Xiaoyu;

#[async_trait]
impl SearchPlugin for Xiaoyu {
    fn name(&self) -> &'static str { "xiaoyu" }
    fn priority(&self) -> i32 { 3 }
    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let mut out = Vec::new();
        // 前 3 页并发
        let mut futs = Vec::new();
        for page in 1..=3 {
            let url = format!("https://xykmovie.com/s/{}/{}", page, q(keyword));
            let c = client.clone();
            futs.push(async move { c.get_text(&url, Some(browser_headers(Some("https://xykmovie.com/")))).await.ok() });
        }
        for body in futures::future::join_all(futs).await.into_iter().flatten() {
            out.extend(xy_parse(&body));
        }
        // 去重
        let mut seen = std::collections::HashSet::new();
        out.retain(|r| seen.insert(r.unique_id.clone()));
        Ok(out)
    }
}
