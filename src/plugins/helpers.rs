//! 插件共享辅助函数：网盘类型映射、密码提取、URL 提取、解密、HTML 清理等。

use crate::model::Link;
use crate::util::netdisk::get_link_type;
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use once_cell::sync::Lazy;
use regex::Regex;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// 按域名/名称确定网盘类型（覆盖插件里各种 convert 函数）
pub fn determine_type(text: &str) -> &'static str {
    let s = text.to_lowercase();
    if s.contains("pan.quark.cn") || s.contains("qualk.cn") || s.contains("夸克") { return "quark"; }
    if s.contains("pan.baidu.com") || s.contains("百度") { return "baidu"; }
    if s.contains("alipan.com") || s.contains("aliyundrive.com") || s.contains("阿里") { return "aliyun"; }
    if s.contains("drive.uc.cn") || s.contains("uc.cn") || s.contains("uc网盘") { return "uc"; }
    if s.contains("cloud.189.cn") || s.contains("天翼") { return "tianyi"; }
    if s.contains("caiyun.139.com") || s.contains("yun.139.com") || s.contains("feixin.10086.cn") || s.contains("移动") || s.contains("彩云") { return "mobile"; }
    if s.contains("115.com") || s.contains("115cdn.com") || s.contains("anxia.com") { return "115"; }
    if s.contains("pan.xunlei.com") || s.contains("迅雷") { return "xunlei"; }
    if s.contains("123pan") || s.contains("123684") || s.contains("123685") || s.contains("123912") || s.contains("123592") || s.contains("123865") { return "123"; }
    if s.contains("mypikpak.com") || s.contains("pikpak") { return "pikpak"; }
    if s.contains("lanzou") { return "lanzou"; }
    if s.contains("magnet:") { return "magnet"; }
    if s.contains("ed2k://") { return "ed2k"; }
    // 兜底用通用识别
    let t = get_link_type(text);
    if t == "others" { "others" } else { t }
}

/// 从 URL 提取 pwd/password/passcode/code 参数
pub fn url_password(url: &str) -> String {
    if let Ok(u) = url::Url::parse(url) {
        for (k, v) in u.query_pairs() {
            if matches!(k.as_ref(), "pwd" | "password" | "passcode" | "code" | "passwd") {
                let val = v.as_ref();
                if (4..=10).contains(&val.len()) { return val.to_string(); }
            }
        }
    }
    // 正则兜底（url 可能带中文/特殊字符导致 parse 失败）
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"[?&](?:pwd|password|passcode|code|passwd)=([0-9a-zA-Z]{4,10})(?:[&#]|$)"#).unwrap());
    RE.captures(url).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default()
}

/// 从链接周边文本提取密码（提取码/密码/pwd/code）
pub fn password_near(context: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(
        r"(?i)(?:提取码|访问码|密码|pwd|code)[：:\s]*([0-9a-zA-Z]{4,10})(?:[^0-9a-zA-Z]|$)"
    ).unwrap());
    RE.captures(context).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default()
}

/// 从文本中提取 http(s)/magnet/ed2k URL（遇常见分隔符截止）
pub fn extract_urls(text: &str) -> Vec<String> {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(
        r#"(?i)(https?://[^\s<>"')\]}，。；;,]+|magnet:\?xt=urn:btih:[0-9a-zA-Z]+[^\s<>"')\]}，。；;,]*|ed2k://\|[^\s<>"']+\|/?)"#
    ).unwrap());
    let mut out = Vec::new();
    for m in RE.find_iter(text) {
        let u = m.as_str().trim_end_matches(|c: char| matches!(c, '。' | '，' | '；' | ')' | '）' | '>' | ','));
        if !out.iter().any(|x: &String| x == u) { out.push(u.to_string()); }
    }
    out
}

/// 给定文本，构造网盘链接列表（类型识别 + 密码关联）
pub fn links_with_type(text: &str) -> Vec<Link> {
    let urls = extract_urls(text);
    let mut out = Vec::new();
    for url in urls {
        let ty = determine_type(&url);
        if ty == "others" && !url.starts_with("magnet:") && !url.starts_with("ed2k:") { continue; }
        let pwd = {
            let p = url_password(&url);
            if !p.is_empty() { p }
            else {
                let pos = text.find(&url).unwrap_or(0);
                let start = pos.saturating_sub(60);
                let end = (pos + url.len() + 60).min(text.len());
                password_near(&text[start..end])
            }
        };
        out.push(Link { r#type: ty.to_string(), url, password: pwd, datetime: None, work_title: None });
    }
    out
}

/// HTML 实体解码 + 去标签（轻量）
pub fn clean_html(html: &str) -> String {
    let mut s = html
        .replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n")
        .replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
        .replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'")
        .replace("&nbsp;", " ").replace("\\/", "/");
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").unwrap());
    s = TAG.replace_all(&s, "").to_string();
    // 去控制字符
    s.chars().filter(|c| !c.is_control() || *c == '\n').collect()
}

/// 去 HTML 标签（保留文本）
pub fn strip_tags(s: &str) -> String {
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").unwrap());
    let v = TAG.replace_all(s, "").to_string();
    crate::util::text::clean_text(&v)
}

/// 关键词分词 AND 匹配（所有插件通用过滤）
pub fn matches_keyword(title: &str, content: &str, keyword: &str) -> bool {
    crate::util::text::matches_keywords(title, content, keyword)
}

/// PKCS7 去填充
fn unpad(data: &[u8]) -> Option<&[u8]> {
    if data.is_empty() { return None; }
    let n = *data.last().unwrap() as usize;
    if n == 0 || n > data.len() || n > 16 { return None; }
    let (body, pad) = data.split_at(data.len() - n);
    if pad.iter().all(|b| *b as usize == n) { Some(body) } else { None }
}

/// AES-128-CBC 固定 key/iv 解密（sdso 用）：返回解密后的明文字符串
pub fn aes_cbc_decrypt_b64(b64: &str, key: &[u8; 16], iv: &[u8; 16]) -> anyhow::Result<String> {
    use base64::Engine;
    let ct = base64::engine::general_purpose::STANDARD.decode(b64.trim())?;
    if ct.len() % 16 != 0 { anyhow::bail!("密文长度非法"); }
    let cipher = Aes128CbcDec::new(key.into(), iv.into());
    let mut buf = ct;
    let pt = cipher.decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow::anyhow!("解密失败: {:?}", e))?;
    let _ = unpad(pt); // decrypt_padded_mut 已去填充
    Ok(String::from_utf8_lossy(pt).to_string())
}

/// XOR 解密（yunso 用）：base64 -> 若以 http 开头直接返回，否则逐字节 XOR key
pub fn xor_decrypt_b64(value: &str, key: &str) -> String {
    use base64::Engine;
    let mut s = value.to_string();
    while s.len() % 4 != 0 { s.push('='); }
    let decoded = match base64::engine::general_purpose::STANDARD.decode(&s) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let raw = String::from_utf8_lossy(&decoded).to_string();
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("magnet:") {
        return raw;
    }
    let kb = key.as_bytes();
    let out: Vec<u8> = decoded.iter().enumerate().map(|(i, b)| b ^ kb[i % kb.len()]).collect();
    String::from_utf8_lossy(&out).to_string()
}

/// 向链接列表添加一条网盘链接（自动识别类型、密码、去重）
pub fn push_link(out: &mut Vec<Link>, url: &str) {
    let url = url.trim();
    if url.is_empty() { return; }
    let ty = determine_type(url);
    if ty == "others" && !url.starts_with("magnet:") && !url.starts_with("ed2k:") { return; }
    if out.iter().any(|l| l.url == url) { return; }
    out.push(Link {
        r#type: ty.to_string(),
        url: url.to_string(),
        password: url_password(url),
        datetime: None,
        work_title: None,
    });
}

/// 可从 Html 或 ElementRef 中选择节点的统一 trait
pub trait Selectable {
    fn select_all<'a>(&'a self, selector: &scraper::Selector) -> Vec<scraper::ElementRef<'a>>;
}
impl Selectable for scraper::Html {
    fn select_all<'a>(&'a self, selector: &scraper::Selector) -> Vec<scraper::ElementRef<'a>> {
        self.select(selector).collect()
    }
}
impl Selectable for scraper::ElementRef<'_> {
    fn select_all<'a>(&'a self, selector: &scraper::Selector) -> Vec<scraper::ElementRef<'a>> {
        self.select(selector).collect()
    }
}

/// scraper 选择器辅助（同时支持 Html 与 ElementRef）
pub fn sel<'a, N: Selectable>(node: &'a N, s: &str) -> Vec<scraper::ElementRef<'a>> {
    match scraper::Selector::parse(s) {
        Ok(selector) => node.select_all(&selector),
        Err(_) => vec![],
    }
}

/// 取第一个匹配元素的文本
pub fn first_text<N: Selectable>(node: &N, s: &str) -> String {
    sel(node, s).into_iter().next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default()
}

/// 从 JSON Value 提取 id（字符串或数字）
pub fn json_id(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 解析常见时间格式，失败返回 now
pub fn parse_time(s: &str) -> chrono::DateTime<chrono::Utc> {
    let s = s.trim();
    if s.is_empty() { return chrono::Utc::now(); }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) { return dt.with_timezone(&chrono::Utc); }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y/%m/%d", "%Y-%m-%d", "%Y年%m月%d日"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc);
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, fmt) {
            return chrono::DateTime::from_naive_utc_and_offset(d.and_hms_opt(0,0,0).unwrap(), chrono::Utc);
        }
    }
    chrono::Utc::now()
}
