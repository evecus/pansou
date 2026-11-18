//! 网盘链接类型识别、提取与清洗

use once_cell::sync::Lazy;
use regex::Regex;

/// 识别网盘类型（与 Go GetLinkType 一致）
pub fn get_link_type(url: &str) -> &'static str {
    let mut u = url.to_lowercase();
    if u.contains("链接：") || u.contains("链接:") {
        if let Some(pos) = u.find("链接") {
            u = u[pos + 2..].to_string();
            u = u.trim_start_matches(|c| c == '：' || c == ':').trim().to_string();
        }
    }
    if u.contains("ed2k:") { return "ed2k"; }
    if u.starts_with("magnet:") { return "magnet"; }
    if u.contains("pan.baidu.com") { return "baidu"; }
    if u.contains("pan.quark.cn") { return "quark"; }
    if u.contains("alipan.com") || u.contains("aliyundrive.com") { return "aliyun"; }
    if u.contains("guangyapan.com") { return "guangya"; }
    if u.contains("cloud.189.cn") { return "tianyi"; }
    if u.contains("drive.uc.cn") { return "uc"; }
    if u.contains("caiyun.139.com") || u.contains("yun.139.com") || u.contains("caiyun.feixin.10086.cn") { return "mobile"; }
    if u.contains("115.com") || u.contains("115cdn.com") || u.contains("anxia.com") { return "115"; }
    if u.contains("mypikpak.com") { return "pikpak"; }
    if u.contains("pan.xunlei.com") { return "xunlei"; }
    if u.contains("123684.com") || u.contains("123685.com") || u.contains("123865.com") ||
       u.contains("123912.com") || u.contains("123pan.com") || u.contains("123pan.cn") || u.contains("123592.com") {
        return "123";
    }
    "others"
}

static ALL_PAN_LINKS: Lazy<Regex> = Lazy::new(|| Regex::new(
    r#"(?i)(?:(?:magnet:\?xt=urn:btih:[a-zA-Z0-9]+)|(?:ed2k://\|file\|[^|]+\|\d+\|[A-Fa-f0-9]+\|/?)|(?:https?://(?:(?:[\w.-]+\.)?(?:pan\.(?:baidu|quark)\.cn|(?:www\.)?(?:alipan|aliyundrive)\.com|drive\.uc\.cn|cloud\.189\.cn|(?:www\.)?(?:yun|caiyun)\.139\.com|caiyun\.feixin\.10086\.cn|(?:www\.)?123(?:684|685|912|pan|592)\.(?:com|cn)|115\.com|115cdn\.com|anxia\.com|pan\.xunlei\.com|mypikpak\.com|guangyapan\.com))(?:/[^\s'"<>()]*)?))"#
).unwrap());

static PASSWORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"(?i)(?:(?:提取|访问|提取密|密)码|pwd)[：:]\s*([a-zA-Z0-9]{4})(?:[^a-zA-Z0-9]|$)"
).unwrap());
static URL_PASSWORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"(?i)[?&]pwd=([a-zA-Z0-9]{4})(?:[^a-zA-Z0-9]|$)"
).unwrap());

/// 从文本中提取所有网盘链接
pub fn extract_netdisk_links(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for m in ALL_PAN_LINKS.find_iter(text) {
        let s = m.as_str().trim_end_matches(|c| c == '。' || c == '，' || c == ')').to_string();
        if seen.insert(s.clone()) { result.push(s); }
    }
    result
}

/// 从链接周边文本提取提取码
pub fn extract_password(content: &str, url: &str) -> String {
    // 优先从 URL 参数取
    if let Some(c) = URL_PASSWORD_RE.captures(url) {
        return c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    // 在内容中定位 URL，取其前后文本（按 UTF-8 字符边界切片）
    let mut context = content;
    if let Some(pos) = content.find(url) {
        let start = floor_char_boundary(content, pos.saturating_sub(120));
        let end = ceil_char_boundary(content, (pos + url.len() + 120).min(content.len()));
        context = &content[start..end];
    }
    if let Some(c) = PASSWORD_RE.captures(context) {
        return c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    String::new()
}

/// 向下对齐到合法的 UTF-8 字符边界
pub fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() { return s.len(); }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) { i -= 1; }
    i
}

/// 向上对齐到合法的 UTF-8 字符边界
pub fn ceil_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() { return s.len(); }
    let mut i = idx;
    while i < s.len() && !s.is_char_boundary(i) { i += 1; }
    i
}

/// 清洗网盘 URL 尾部杂质
pub fn clean_url(url: &str) -> String {
    let mut u = url.trim().to_string();
    for marker in [" ", "\n", "\t", "，", "。", "；", ";", ",", "〈", "》", "\"", "'"] {
        if let Some(pos) = u.find(marker) {
            if pos > 8 { u = u[..pos].to_string(); }
        }
    }
    u.trim_end_matches(|c: char| c == ')' || c == '）' || c == '>' ).to_string()
}
