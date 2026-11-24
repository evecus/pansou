//! Telegram 公开频道搜索 - 解析 t.me/s/{channel} 页面

use crate::model::{Link, SearchResult};
use crate::util::netdisk::{extract_netdisk_links, extract_password, get_link_type, clean_url};
use crate::util::text::{strip_html, clean_text};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use scraper::{Html, Selector};

static BG_IMAGE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"background-image:url\(['"]?([^'")]+)['"]?\)"#).unwrap());

/// 构建搜索 URL
pub fn build_search_url(channel: &str, keyword: &str, next_page: &str) -> String {
    let mut url = format!("https://t.me/s/{}", channel);
    if !keyword.is_empty() {
        url.push_str("?q=");
        url.push_str(&crate::util::text::url_encode(keyword));
        if !next_page.is_empty() {
            url.push('&');
            url.push_str(next_page);
        }
    }
    url
}

/// 解析 TG 搜索结果页
pub fn parse_search_results(html: &str, channel: &str) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let wrap_sel = Selector::parse(".tgme_widget_message_wrap").unwrap();
    let msg_sel = Selector::parse(".tgme_widget_message").unwrap();
    let date_sel = Selector::parse(".tgme_widget_message_date time").unwrap();
    let text_sel = Selector::parse(".tgme_widget_message_text").unwrap();
    let a_sel = Selector::parse("a").unwrap();
    let photo_sel = Selector::parse(".tgme_widget_message_photo_wrap").unwrap();
    let photo_img_sel = Selector::parse(".tgme_widget_message_photo img").unwrap();

    let mut results = Vec::new();

    for wrap in doc.select(&wrap_sel) {
        let msg = match wrap.select(&msg_sel).next() {
            Some(m) => m,
            None => continue,
        };
        let data_post = match msg.value().attr("data-post") {
            Some(v) => v,
            None => continue,
        };
        let parts: Vec<&str> = data_post.split('/').collect();
        if parts.len() != 2 { continue; }
        let message_id = parts[1];
        let unique_id = format!("{}_{}", channel, message_id);

        let time_str = match msg.select(&date_sel).next().and_then(|t| t.value().attr("datetime")) {
            Some(v) => v,
            None => continue,
        };
        let datetime = parse_rfc3339(time_str).unwrap_or_else(Utc::now);

        let text_elem = msg.select(&text_sel).next();
        let (message_html, message_text) = match text_elem {
            Some(el) => (
                el.inner_html(),
                el.text().collect::<String>(),
            ),
            None => (String::new(), String::new()),
        };
        let title = extract_title(&message_html, &message_text);

        // 提取链接
        let mut links: Vec<Link> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // 来自 <a> 标签
        if let Some(el) = text_elem {
            for a in el.select(&a_sel) {
                if let Some(href) = a.value().attr("href") {
                    let lt = get_link_type(href);
                    if lt == "others" && !href.starts_with("magnet:") && !href.starts_with("ed2k:") { continue; }
                    let password = extract_password(&message_text, href);
                    let url = clean_url(href);
                    if seen.insert(url.clone()) {
                        links.push(Link { r#type: lt.to_string(), url, password, datetime: None, work_title: None });
                    }
                }
            }
        }
        // 来自纯文本
        for link_url in extract_netdisk_links(&message_text) {
            let lt = get_link_type(&link_url);
            let password = extract_password(&message_text, &link_url);
            let url = clean_url(&link_url);
            if seen.insert(url.clone()) {
                links.push(Link { r#type: lt.to_string(), url, password, datetime: None, work_title: None });
            }
        }

        // 提取图片
        let mut images: Vec<String> = Vec::new();
        let mut seen_img = std::collections::HashSet::new();
        for ph in msg.select(&photo_sel) {
            if let Some(style) = ph.value().attr("style") {
                if let Some(c) = BG_IMAGE_RE.captures(style) {
                    let u = c.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
                    if !u.is_empty() && seen_img.insert(u.clone()) { images.push(u); }
                }
            }
        }
        for img in msg.select(&photo_img_sel) {
            if let Some(src) = img.value().attr("src") {
                if !src.is_empty() && seen_img.insert(src.to_string()) { images.push(src.to_string()); }
            }
        }

        results.push(SearchResult {
            message_id: message_id.to_string(),
            unique_id,
            channel: channel.to_string(),
            datetime,
            title,
            content: clean_text(&message_text),
            links,
            tags: vec![],
            images,
        });
    }
    results
}

fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn extract_title(html: &str, text: &str) -> String {
    // <br> 前的第一行
    if let Some(pos) = html.find("<br") {
        if pos > 0 {
            let first_line = strip_html(&html[..pos]);
            let first_line = first_line.trim();
            if let Some(rest) = first_line.strip_prefix("名称：") {
                return rest.trim().to_string();
            }
            if !first_line.starts_with('#') {
                return cut_title(first_line);
            }
        }
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() { return String::new(); }
    let mut first_line = lines[0].trim();
    if first_line.starts_with('#') {
        for line in &lines {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("名称：") { return rest.trim().to_string(); }
        }
        if lines.len() > 1 {
            let second = lines[1].trim();
            if let Some(rest) = second.strip_prefix("名称：") { return rest.trim().to_string(); }
            if !second.is_empty() && !second.starts_with('#') { return cut_title(second); }
        }
    }
    if let Some(rest) = first_line.strip_prefix("名称：") { return rest.trim().to_string(); }
    first_line = first_line.trim();
    cut_title(first_line)
}

fn cut_title(title: &str) -> String {
    let mut end = title.len();
    for kw in ["简介", "描述"] {
        if let Some(pos) = title.find(kw) { end = end.min(pos); }
    }
    title[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_realistic() {
        let html = r#"<div class="tgme_widget_message_wrap"><div class="tgme_widget_message" data-post="chan/123"><div class="tgme_widget_message_text">名称：爱情公寓 简介：xx<br>链接：<a href="https://pan.quark.cn/s/abc">https://pan.quark.cn/s/abc</a><br>提取码：abcd 📁 大小：1G</div><a class="tgme_widget_message_date"><time datetime="2026-11-24T00:00:00+00:00">x</time></a></div></div>
        <div class="tgme_widget_message_wrap"><div class="tgme_widget_message" data-post="chan/124"><div class="tgme_widget_message_text"><a href="链接：https://pan.quark.cn/s/abc">爱情公寓</a> 🔥描述：好看</div><a class="tgme_widget_message_date"><time datetime="2026-11-24T00:00:00+00:00">x</time></a></div></div>"#;
        let r = parse_search_results(html, "chan");
        assert!(!r.is_empty());
    }
}
