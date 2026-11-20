//! 文本处理工具：HTML 清理、关键词过滤、标题提取等

use once_cell::sync::Lazy;
use regex::Regex;

static HTML_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").unwrap());
static BR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]+").unwrap());
static NL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());

/// 剥除 HTML 标签并解码常见实体
pub fn strip_html(html: &str) -> String {
    let s = BR_RE.replace_all(html, "\n");
    let s = HTML_TAG_RE.replace_all(&s, "");
    decode_entities(&s)
}

/// 解码常见 HTML 实体
pub fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<")
        .replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
        .replace("&ldquo;", "\u{201c}").replace("&rdquo;", "\u{201d}")
        .replace("&mdash;", "—").replace("&hellip;", "…")
        .replace("&#x27;", "'")
}

/// 清理文本
pub fn clean_text(s: &str) -> String {
    let s = strip_html(s);
    let s = WS_RE.replace_all(&s, " ");
    let s = NL_RE.replace_all(&s, "\n\n");
    s.trim().to_string()
}

/// 按关键词过滤（与 Go FilterResultsByKeyword 一致）：所有关键词（空格分隔）都需命中标题或内容
pub fn matches_keywords(title: &str, content: &str, keyword: &str) -> bool {
    if keyword.trim().is_empty() { return true; }
    let lower_kw = keyword.to_lowercase();
    let lt = title.to_lowercase();
    let lc = content.to_lowercase();
    for kw in lower_kw.split_whitespace() {
        if !lt.contains(kw) && !lc.contains(kw) { return false; }
    }
    true
}

/// URL 编码（简单实现，覆盖常见字符）
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
