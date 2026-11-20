//! 网盘链接检测服务：移植 Go check_service.go
//! 支持 baidu/quark/aliyun/uc/tianyi/123/xunlei/115/mobile 九种网盘，
//! 串行检测、按状态分级 TTL 缓存、代理隔离、URL 规范化。

use crate::model::{CheckItem, CheckRequest, CheckResponse, CheckResult};
use crate::util::http::build_client;
use crate::util::HttpClient;
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use md5::{Digest, Md5};
use rand::RngCore;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ============ 状态常量与 TTL ============
const STATE_OK: &str = "ok";
const STATE_BAD: &str = "bad";
const STATE_LOCKED: &str = "locked";
const STATE_UNSUPPORTED: &str = "unsupported";
const STATE_UNCERTAIN: &str = "uncertain";

fn ttl_for_state(state: &str) -> Duration {
    match state {
        STATE_OK => Duration::from_secs(24 * 3600),
        STATE_BAD => Duration::from_secs(6 * 3600),
        STATE_LOCKED => Duration::from_secs(12 * 3600),
        STATE_UNSUPPORTED => Duration::from_secs(24 * 3600),
        _ => Duration::from_secs(30 * 60),
    }
}

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
const MOBILE_KEY: &[u8; 16] = b"PVGDwmcvfs1uV3d1";

/// 检测缓存条目
#[derive(Clone)]
struct CachedEntry {
    result: CheckResult,
    expires_at_ms: i64,
}

pub struct CheckService {
    client: HttpClient,
    cache: Arc<Mutex<HashMap<String, CachedEntry>>>,
}

impl Default for CheckService {
    fn default() -> Self { Self::new() }
}

impl CheckService {
    pub fn new() -> Self {
        Self {
            client: HttpClient::new(build_client(None, Duration::from_secs(60)), 0),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 主入口：处理整个请求（代理优先级 proxy_url > proxy）
    pub async fn check(&self, req: &CheckRequest) -> CheckResponse {
        let proxy = if !req.proxy_url.trim().is_empty() {
            req.proxy_url.trim()
        } else {
            req.proxy.trim()
        };

        let client = if proxy.is_empty() {
            self.client.clone()
        } else {
            match build_client(Some(proxy), Duration::from_secs(60)) {
                c => HttpClient::new(c, 0),
            }
        };
        let cache_scope = if proxy.is_empty() {
            String::new()
        } else {
            let mut h = Md5::new();
            h.update(proxy.as_bytes());
            format!("proxy:{}", hex_encode(&h.finalize()))
        };

        let mut results = Vec::with_capacity(req.items.len());
        for item in &req.items {
            results.push(self.check_one(item, &client, &cache_scope).await);
        }
        CheckResponse { results }
    }

    /// 检测单个链接
    async fn check_one(&self, item: &CheckItem, client: &HttpClient, cache_scope: &str) -> CheckResult {
        let normalized = normalize_share_link(&item.disk_type, &item.url, &item.password);
        if normalized.is_empty() {
            return build_result(item, "", STATE_UNCERTAIN, false, "链接格式无效");
        }

        let cache_key = format!("{}|{}{}", item.disk_type, normalized,
            if cache_scope.is_empty() { String::new() } else { format!("|{}", cache_scope) });

        // 查缓存
        if let Some(entry) = self.cache.lock().unwrap().get(&cache_key) {
            let now = chrono::Utc::now().timestamp_millis();
            if now < entry.expires_at_ms {
                let mut r = entry.result.clone();
                r.cache_hit = true;
                return r;
            }
        }

        // 执行检测
        let (state, summary) = match self.run_check(item, &normalized, client).await {
            Ok(v) => v,
            Err(e) => (STATE_UNCERTAIN, format!("检测失败: {}", e)),
        };

        let result = build_result(item, &normalized, state, false, &summary);
        // 写缓存
        let expires = chrono::Utc::now().timestamp_millis() + ttl_for_state(state).as_millis() as i64;
        self.cache.lock().unwrap().insert(cache_key, CachedEntry {
            result: result.clone(),
            expires_at_ms: expires,
        });
        result
    }

    /// 分派到具体平台检测
    async fn run_check(&self, item: &CheckItem, normalized: &str, client: &HttpClient)
        -> anyhow::Result<(&'static str, String)>
    {
        match item.disk_type.as_str() {
            "aliyun" => check_aliyun(client, normalized).await,
            "quark" => check_quark(client, normalized).await,
            "uc" => check_uc(client, normalized).await,
            "baidu" => check_baidu(client, normalized, &item.password).await,
            "tianyi" => check_tianyi(client, normalized, &item.password).await,
            "123" => check_123(client, normalized).await,
            "xunlei" => check_xunlei(client, normalized).await,
            "115" => check_115(client, normalized, &item.password).await,
            "mobile" => check_mobile(client, normalized, &item.password).await,
            _ => Ok((STATE_UNSUPPORTED, "当前平台暂不支持检测".into())),
        }
    }
}

// ============ URL 规范化 ============
fn normalize_share_link(disk_type: &str, raw: &str, password: &str) -> String {
    let base = raw.trim();
    if base.is_empty() { return String::new(); }
    let mut parsed = match url::Url::parse(base) {
        Ok(p) => p,
        Err(_) => return base.to_string(),
    };
    parsed.set_fragment(None);
    // 重建 query（排序 + 可选补 pwd）
    let mut pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
    let has_pwd = pairs.iter().any(|(k, _)| k == "pwd");
    if !password.is_empty() && !has_pwd && matches!(disk_type, "baidu" | "quark" | "uc") {
        pairs.push(("pwd".into(), password.into()));
    }
    if !pairs.is_empty() {
        let mut ser = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &pairs { ser.append_pair(k, v); }
        parsed.set_query(Some(&ser.finish()));
    } else {
        parsed.set_query(None);
    }
    // host 小写
    if let Some(host) = parsed.host_str() {
        let lower = host.to_lowercase();
        if host != lower {
            let port = parsed.port();
            let _ = parsed.set_host(Some(&lower));
            if let Some(p) = port { let _ = parsed.set_port(Some(p)); }
        }
    }
    parsed.to_string()
}

// ============ 结果构建 ============
fn build_result(item: &CheckItem, normalized: &str, state: &str, cache_hit: bool, summary: &str) -> CheckResult {
    let now = chrono::Utc::now().timestamp_millis();
    let expires = now + ttl_for_state(state).as_millis() as i64;
    CheckResult {
        disk_type: item.disk_type.clone(),
        url: item.url.clone(),
        normalized_url: if normalized.is_empty() { None } else { Some(normalized.to_string()) },
        state: state.to_string(),
        cache_hit,
        checked_at: now,
        expires_at: expires,
        summary: if summary.is_empty() { None } else { Some(summary.to_string()) },
    }
}

// ============ 通用 header 辅助 ============
fn hdr(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            h.insert(name, val);
        }
    }
    h
}

fn contains_any(s: &str, kws: &[&str]) -> bool {
    let lower = s.to_lowercase();
    kws.iter().any(|k| lower.contains(&k.to_lowercase()))
}

fn json_field<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for k in keys {
        if let Some(x) = v.get(*k) { return Some(x); }
    }
    None
}

fn as_str(v: Option<&Value>) -> &str {
    v.and_then(|x| x.as_str()).unwrap_or("")
}

// ============ 阿里云盘 ============
async fn check_aliyun(client: &HttpClient, normalized: &str) -> anyhow::Result<(&'static str, String)> {
    let share_id = normalized.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if share_id.is_empty() { return Ok((STATE_BAD, "share_id 为空".into())); }
    let url = format!("https://api.aliyundrive.com/adrive/v3/share_link/get_share_by_anonymous?share_id={}", share_id);
    let headers = hdr(&[
        ("content-type", "application/json"),
        ("origin", "https://www.alipan.com"),
        ("referer", "https://www.alipan.com/"),
        ("x-canary", "client=web,app=share,version=v2.3.1"),
    ]);
    let (_, body, _) = client.request_text(reqwest::Method::POST, &url, Some(headers),
        Some(json!({"share_id": share_id}).to_string())).await?;
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let code = as_str(json_field(&v, &["code"]));
    if contains_any(code, &["sharelink", "notfound", "not_found", "cancelled", "canceled", "forbidden", "expired"]) {
        return Ok((STATE_BAD, format!("分享失效: {}", code)));
    }
    if contains_any(code, &["exceed", "frequency", "limit"]) {
        return Ok((STATE_UNCERTAIN, "请求受限".into()));
    }
    let file_count = json_field(&v, &["file_count"]).and_then(|x| x.as_i64()).unwrap_or(0);
    let share_name = as_str(json_field(&v, &["share_name"]));
    let share_title = as_str(json_field(&v, &["share_title"]));
    let status = as_str(json_field(&v, &["status"]));
    if file_count == 0 && share_name.is_empty() && share_title.is_empty() {
        return Ok((STATE_BAD, "分享内容为空".into()));
    }
    if !status.is_empty() && !matches!(status, "enabled" | "normal") &&
        contains_any(status, &["forbidden", "cancel", "expired", "illegal", "invalid", "disabled"]) {
        return Ok((STATE_BAD, format!("分享状态异常: {}", status)));
    }
    if !share_name.is_empty() || !share_title.is_empty() || file_count > 0 {
        return Ok((STATE_OK, "分享有效".into()));
    }
    Ok((STATE_UNCERTAIN, "无法确认分享状态".into()))
}

// ============ 夸克网盘 ============
async fn check_quark(client: &HttpClient, normalized: &str) -> anyhow::Result<(&'static str, String)> {
    let (resource_id, pwd) = extract_quark(normalized);
    if resource_id.is_empty() { return Ok((STATE_BAD, "未找到分享ID".into())); }

    let token_url = "https://drive-h.quark.cn/1/clouddrive/share/sharepage/token";
    let headers = hdr(&[
        ("content-type", "application/json"),
        ("origin", "https://pan.quark.cn"),
        ("referer", "https://pan.quark.cn/"),
    ]);
    let body_json = json!({
        "pwd_id": resource_id,
        "passcode": pwd,
        "support_visit_limit_private_share": true
    }).to_string();
    let (_, body, _) = client.request_text(reqwest::Method::POST, token_url, Some(headers), Some(body_json)).await?;
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let code = json_field(&v, &["code"]).and_then(|x| x.as_i64()).unwrap_or(-1);
    let message = as_str(json_field(&v, &["message"]));
    if code != 0 {
        return Ok((classify_quark_code(code, message), classify_quark_msg(message)));
    }
    let stoken = as_str(json_field(&v, &["data", "stoken"])).to_string();
    if stoken.is_empty() {
        return Ok((STATE_UNCERTAIN, "未获取到 stoken".into()));
    }

    let detail_url = format!("https://drive-pc.quark.cn/1/clouddrive/share/sharepage/detail?pwd_id={}&stoken={}&ver=2&pr=ucpro",
        resource_id, stoken);
    let dheaders = hdr(&[
        ("accept", "application/json, text/plain, */*"),
        ("origin", "https://pan.quark.cn"),
        ("referer", "https://pan.quark.cn/"),
        ("cache-control", "no-cache"),
    ]);
    let (_, dbody, _) = client.request_text(reqwest::Method::GET, &detail_url, Some(dheaders), None).await?;
    let d: Value = serde_json::from_str(&dbody).unwrap_or(Value::Null);
    let dcode = json_field(&d, &["code"]).and_then(|x| x.as_i64()).unwrap_or(-1);
    let dmsg = as_str(json_field(&d, &["message"]));
    if dcode != 0 {
        return Ok((classify_quark_code(dcode, dmsg), classify_quark_msg(dmsg)));
    }
    let data = json_field(&d, &["data"]).unwrap_or(&Value::Null);
    let list = data.get("list").and_then(|x| x.as_array());
    let status = data.get("share").and_then(|s| s.get("status")).and_then(|x| x.as_i64()).unwrap_or(0);
    let partial = data.get("share").and_then(|s| s.get("partial_violation"))
        .or_else(|| data.get("share").and_then(|s| s.get("PartialViolation")))
        .and_then(|x| x.as_bool()).unwrap_or(false);
    let is_expire = data.get("share").and_then(|s| s.get("is_expire"))
        .and_then(|x| x.as_bool()).unwrap_or(false);

    match list {
        Some(arr) if !arr.is_empty() => {
            if status == 1 && partial {
                Ok((STATE_OK, "有效，部分文件违规".into()))
            } else if status == 1 {
                Ok((STATE_OK, "分享有效".into()))
            } else if status == 3 && partial {
                Ok((STATE_BAD, "因违规已失效".into()))
            } else if status == 3 {
                Ok((STATE_OK, "分享有效".into()))
            } else if status > 1 {
                Ok((STATE_BAD, "分享已失效".into()))
            } else if partial {
                Ok((STATE_OK, "有效，部分文件违规".into()))
            } else {
                Ok((STATE_OK, "分享有效".into()))
            }
        }
        _ => {
            if is_expire { Ok((STATE_BAD, "分享已过期".into())) }
            else if status > 1 { Ok((STATE_BAD, "分享已失效".into())) }
            else { Ok((STATE_BAD, "分享文件列表为空".into())) }
        }
    }
}

fn classify_quark_code(code: i64, msg: &str) -> &'static str {
    if code == 41008 || contains_any(msg, &["提取码", "密码", "passcode"]) { STATE_LOCKED }
    else if contains_any(msg, &["不存在", "失效", "违规", "过期", "取消", "deleted", "expired"]) { STATE_BAD }
    else { STATE_UNCERTAIN }
}
fn classify_quark_msg(msg: &str) -> String {
    if msg.is_empty() { "分享不可用".into() } else { msg.to_string() }
}

fn extract_quark(url: &str) -> (String, String) {
    let re = regex::Regex::new(r"/s/([A-Za-z0-9]+)").unwrap();
    let id = re.captures(url).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
    let pwd = url::Url::parse(url).ok()
        .and_then(|u| u.query_pairs().into_owned().find(|(k, _)| k == "pwd").map(|(_, v)| v))
        .unwrap_or_default();
    (id, pwd)
}

// ============ UC 网盘 ============
async fn check_uc(client: &HttpClient, normalized: &str) -> anyhow::Result<(&'static str, String)> {
    let headers = hdr(&[("user-agent", "Mozilla/5.0 (Linux; Android 10; Mobile) AppleWebKit/537.36 Chrome/124.0 Safari/537.36")]);
    let (status, body, _) = client.request_text(reqwest::Method::GET, normalized, Some(headers), None).await?;
    if status == 404 { return Ok((STATE_BAD, "页面不存在".into())); }
    let lower = body.to_lowercase();
    if contains_any(&lower, &["失效", "不存在", "违规", "删除", "已过期", "被取消"]) {
        return Ok((STATE_BAD, "分享已失效".into()));
    }
    if contains_any(&lower, &["提取码", "访问码", "请输入密码"]) {
        return Ok((STATE_LOCKED, "需要提取码".into()));
    }
    if contains_any(&lower, &["文件", "分享", "drive.uc.cn"]) {
        return Ok((STATE_OK, "分享有效".into()));
    }
    Ok((STATE_UNCERTAIN, "无法确认分享状态".into()))
}

// ============ 百度网盘 ============
async fn check_baidu(client: &HttpClient, normalized: &str, item_pwd: &str) -> anyhow::Result<(&'static str, String)> {
    let (share_id, mut pwd) = extract_baidu(normalized);
    if pwd.is_empty() { pwd = item_pwd.to_string(); }
    let mut short = share_id.clone();
    if short.starts_with('1') && short.len() > 1 { short = short[1..].to_string(); }
    if short.is_empty() { return Ok((STATE_BAD, "未找到分享ID".into())); }

    let mut cookie = String::new();
    if !pwd.is_empty() {
        let verify_url = format!("https://pan.baidu.com/share/verify?surl={}&pwd={}", short, pwd);
        let vh = hdr(&[
            ("referer", normalized),
            ("content-type", "application/x-www-form-urlencoded"),
        ]);
        let form = format!("pwd={}&vcode=&vcode_str=", pwd);
        let (_, vbody, _) = client.request_text(reqwest::Method::POST, &verify_url, Some(vh), Some(form)).await?;
        let v: Value = serde_json::from_str(&vbody).unwrap_or(Value::Null);
        let errno = json_field(&v, &["errno"]).and_then(|x| x.as_i64()).unwrap_or(-1);
        if errno == 0 {
            if let Some(randsk) = v.get("randsk").and_then(|x| x.as_str()) {
                cookie = format!("BDCLND={}", randsk);
            }
        } else if errno == -9 || errno == -12 {
            return Ok((STATE_LOCKED, "提取码错误或需要提取码".into()));
        } else {
            return Ok((STATE_UNCERTAIN, "提取码验证失败".into()));
        }
    }

    let url = format!("https://pan.baidu.com/share/list?web=1&page=1&num=20&order=time&desc=1&showempty=0&shorturl={}&root=1&clienttype=0", short);
    let mut hvec: Vec<(&str, &str)> = vec![
        ("accept", "application/json,text/plain,*/*"),
        ("referer", normalized),
        ("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/124.0 Safari/537.36"),
    ];
    if !cookie.is_empty() { hvec.push(("cookie", &cookie)); }
    let h = hdr(&hvec);
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(h), None).await?;
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let errno = json_field(&v, &["errno"]).and_then(|x| x.as_i64()).unwrap_or(-1);
    let list_len = v.get("list").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
    if errno == 0 && list_len > 0 { return Ok((STATE_OK, "分享有效".into())); }
    if errno == 0 && list_len == 0 { return Ok((STATE_BAD, "分享内容为空".into())); }
    if errno == -9 || errno == -12 { return Ok((STATE_LOCKED, "需要提取码".into())); }
    if matches!(errno, -7 | 105 | 115 | 117 | 145) { return Ok((STATE_BAD, "分享已失效".into())); }
    Ok((STATE_UNCERTAIN, format!("errno={}", errno)))
}

fn extract_baidu(url: &str) -> (String, String) {
    let mut id = String::new();
    if let Ok(u) = url::Url::parse(url) {
        let segs: Vec<&str> = u.path_segments().map(|s| s.collect()).unwrap_or_default();
        if let Some(pos) = segs.iter().position(|s| *s == "s") {
            if let Some(x) = segs.get(pos + 1) { id = x.to_string(); }
        }
        if id.is_empty() {
            id = u.query_pairs().into_owned().find(|(k, _)| k == "surl").map(|(_, v)| v).unwrap_or_default();
        }
    }
    let pwd = url::Url::parse(url).ok()
        .and_then(|u| u.query_pairs().into_owned().find(|(k, _)| k == "pwd").map(|(_, v)| v))
        .unwrap_or_default();
    (id, pwd)
}

// ============ 天翼云盘 ============
async fn check_tianyi(client: &HttpClient, normalized: &str, item_pwd: &str) -> anyhow::Result<(&'static str, String)> {
    let (code, mut pwd) = extract_tianyi(normalized);
    if pwd.is_empty() { pwd = item_pwd.to_string(); }
    if code.is_empty() { return Ok((STATE_BAD, "未找到分享码".into())); }
    let no_cache = format!("{}", chrono::Utc::now().timestamp_millis());
    let share_code = if pwd.is_empty() {
        code.clone()
    } else {
        format!("{}（访问码：{}）", code, pwd)
    };
    let url = format!("https://cloud.189.cn/api/open/share/getShareInfoByCodeV2.action?noCache={}&shareCode={}",
        no_cache, urlencoding(&share_code));
    let h = hdr(&[("referer", normalized), ("sign-type", "1")]);
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(h), None).await?;
    let lower = body.to_lowercase();

    // XML shareVO
    if lower.contains("<sharevo>") {
        if lower.contains("<shareid>") || lower.contains("<filename>") || lower.contains("<needaccesscode>1</needaccesscode>") {
            return Ok((STATE_OK, "分享有效".into()));
        }
    }
    if lower.contains("needaccesscode") || contains_any(&lower, &["访问码", "提取码", "密码", "erroraccesscode"]) {
        return Ok((STATE_LOCKED, "需要访问码".into()));
    }
    if contains_any(&lower, &["shareinfonotfound", "sharenotfound", "filenotfound", "shareexpirederror",
        "foldernotfound", "shareauditnotpass", "不存在", "失效", "取消", "过期"]) {
        return Ok((STATE_BAD, "分享已失效".into()));
    }
    // JSON 尝试
    if let Ok(v) = serde_json::from_str::<Value>(&body) {
        let share_id = json_field(&v, &["shareId", "share_id"]).and_then(|x| x.as_i64()).unwrap_or(0);
        let file_name = as_str(json_field(&v, &["fileName", "file_name"]));
        let need = json_field(&v, &["needAccessCode"]).and_then(|x| x.as_i64()).unwrap_or(0);
        if share_id > 0 || !file_name.is_empty() || need == 1 {
            return Ok((STATE_OK, "分享有效".into()));
        }
        if !as_str(json_field(&v, &["error_code"])).is_empty() {
            return Ok((STATE_BAD, "分享已失效".into()));
        }
    }
    Ok((STATE_UNCERTAIN, "无法确认分享状态".into()))
}

fn extract_tianyi(url: &str) -> (String, String) {
    let mut code = String::new();
    let mut pwd = String::new();
    if let Ok(u) = url::Url::parse(url) {
        if let Some(c) = u.query_pairs().into_owned().find(|(k, _)| k == "code").map(|(_, v)| v) {
            code = c;
        }
    }
    if code.is_empty() {
        let re1 = regex::Regex::new(r"/t/([^/?#]+)").unwrap();
        code = re1.captures(url).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    if let Some(pos) = code.find('/') { code = code[..pos].to_string(); }
    let re_pwd = regex::Regex::new(r"（访问码[：:]\s*([a-zA-Z0-9]+)）").unwrap();
    if let Some(c) = re_pwd.captures(url) {
        pwd = c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    (code, pwd)
}

// ============ 123 网盘 ============
async fn check_123(client: &HttpClient, normalized: &str) -> anyhow::Result<(&'static str, String)> {
    let key = extract_123(normalized);
    if key.is_empty() { return Ok((STATE_BAD, "未找到分享Key".into())); }
    let url = format!("https://www.123pan.com/api/share/info?shareKey={}", key);
    let (status, body, _) = client.request_text(reqwest::Method::GET, &url, None, None).await?;
    if status == 403 { return Ok((STATE_OK, "分享有效".into())); }
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let code = json_field(&v, &["code"]).and_then(|x| x.as_i64()).unwrap_or(-1);
    if code == 0 { return Ok((STATE_OK, "分享有效".into())); }
    if v.get("data").and_then(|d| d.get("HasPwd")).and_then(|x| x.as_bool()).unwrap_or(false) {
        return Ok((STATE_LOCKED, "需要提取码".into()));
    }
    let msg = as_str(json_field(&v, &["message"]));
    if !msg.is_empty() { return Ok((STATE_BAD, msg.to_string())); }
    Ok((STATE_BAD, "分享不可用".into()))
}

fn extract_123(url: &str) -> String {
    let re = regex::Regex::new(r"(?i)https?://(?:www\.)?(?:123684|123685|123912|123pan|123592|123865)\.com/s/([a-zA-Z0-9-]+)|123pan\.cn/s/([a-zA-Z0-9-]+)").unwrap();
    if let Some(c) = re.captures(url) {
        return c.get(1).or_else(|| c.get(2)).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    url.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

// ============ 迅雷 ============
async fn check_xunlei(client: &HttpClient, normalized: &str) -> anyhow::Result<(&'static str, String)> {
    let (share_id, pwd) = extract_xunlei(normalized);
    if share_id.is_empty() { return Ok((STATE_BAD, "未找到分享ID".into())); }
    let captcha_token = fetch_xunlei_token(client).await.unwrap_or_default();

    let url = format!("https://api-pan.xunlei.com/drive/v1/share?share_id={}&pass_code={}&limit=100&pass_code_token=&page_token=&thumbnail_size=SIZE_SMALL",
        share_id, pwd);
    let h = hdr(&[
        ("accept", "*/*"),
        ("content-type", "application/json"),
        ("origin", "https://pan.xunlei.com"),
        ("referer", "https://pan.xunlei.com/"),
        ("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/140.0.0.0 Safari/537.36"),
        ("x-client-id", "ZUBzD9J_XPXfn7f7"),
        ("x-device-id", "5505bd0cab8c9469b98e5891d9fb3e0d"),
    ]);
    let mut h = h;
    if !captcha_token.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&captcha_token) { h.insert("x-captcha-token", v); }
    }
    let (status, body, _) = client.request_text(reqwest::Method::GET, &url, Some(h), None).await?;
    if status == 404 || status == 403 { return Ok((STATE_BAD, "分享不存在".into())); }
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let share_status = as_str(json_field(&v, &["share_status"]));
    let error = as_str(json_field(&v, &["error", "error_description", "share_statusText"]));
    if contains_any(error, &["pass_code", "提取码", "密码"]) { return Ok((STATE_LOCKED, "需要提取码".into())); }
    if share_status == "OK" { return Ok((STATE_OK, "分享有效".into())); }
    let sid = as_str(json_field(&v, &["share_id"]));
    let sname = as_str(json_field(&v, &["share_name"]));
    let fcount = json_field(&v, &["file_count"]).and_then(|x| x.as_i64()).unwrap_or(0);
    if !sid.is_empty() || !sname.is_empty() || fcount > 0 { return Ok((STATE_OK, "分享有效".into())); }
    if !share_status.is_empty() { return Ok((STATE_BAD, "分享已失效".into())); }
    Ok((STATE_UNCERTAIN, "无法确认分享状态".into()))
}

fn extract_xunlei(url: &str) -> (String, String) {
    let re = regex::Regex::new(r"pan\.xunlei\.com/s/([^?/#]+)").unwrap();
    let id = re.captures(url).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
    let pwd = url::Url::parse(url).ok()
        .and_then(|u| u.query_pairs().into_owned().find(|(k, _)| k == "pwd").map(|(_, v)| v))
        .unwrap_or_default();
    (id, pwd)
}

async fn fetch_xunlei_token(client: &HttpClient) -> anyhow::Result<String> {
    let device_id = "5505bd0cab8c9469b98e5891d9fb3e0d";
    let client_id = "ZUBzD9J_XPXfn7f7";
    let client_version = "1.10.0.2633";
    let package_name = "com.xunlei.browser";
    let timestamp = chrono::Utc::now().timestamp_millis().to_string();
    let signature = build_xunlei_signature(client_id, client_version, package_name, device_id, &timestamp);
    let body = json!({
        "action": "get:/drive/v1/share",
        "captcha_token": "",
        "client_id": client_id,
        "device_id": device_id,
        "meta": {
            "timestamp": timestamp,
            "captcha_sign": signature,
            "client_version": client_version,
            "package_name": package_name
        },
        "redirect_uri": "xlaccsdk01://xunlei.com/callback?state=harbor"
    }).to_string();
    let h = hdr(&[
        ("accept", "application/json;charset=UTF-8"),
        ("content-type", "application/json"),
        ("x-device-id", device_id),
        ("x-client-id", client_id),
        ("x-client-version", client_version),
    ]);
    let (_, b, _) = client.request_text(reqwest::Method::POST,
        "https://xluser-ssl.xunlei.com/v1/shield/captcha/init", Some(h), Some(body)).await?;
    let v: Value = serde_json::from_str(&b).unwrap_or(Value::Null);
    if !as_str(json_field(&v, &["url"])).is_empty() {
        anyhow::bail!("xunlei captcha required");
    }
    Ok(as_str(json_field(&v, &["captcha_token"])).to_string())
}

fn build_xunlei_signature(client_id: &str, client_version: &str, package_name: &str, device_id: &str, timestamp: &str) -> String {
    let mut content = format!("{}{}{}{}{}", client_id, client_version, package_name, device_id, timestamp);
    let salts = [
        "uWRwO7gPfdPB/0NfPtfQO+71",
        "F93x+qPluYy6jdgNpq+lwdH1ap6WOM+nfz8/V",
        "0HbpxvpXFsBK5CoTKam",
        "dQhzbhzFRcawnsZqRETT9AuPAJ+wTQso82mRv",
        "SAH98AmLZLRa6DB2u68sGhyiDh15guJpXhBzI",
        "unqfo7Z64Rie9RNHMOB",
        "7yxUdFADp3DOBvXdz0DPuKNVT35wqa5z0DEyEvf",
        "RBG",
        "ThTWPG5eC0UBqlbQ+04nZAptqGCdpv9o55A",
    ];
    for salt in salts {
        let mut hasher = Md5::new();
        hasher.update(format!("{}{}", content, salt).as_bytes());
        content = hex_encode(&hasher.finalize());
    }
    format!("1.{}", content)
}

// ============ 115 网盘 ============
async fn check_115(client: &HttpClient, normalized: &str, item_pwd: &str) -> anyhow::Result<(&'static str, String)> {
    let (code, mut pwd) = extract_115(normalized);
    if pwd.is_empty() { pwd = item_pwd.to_string(); }
    if code.is_empty() { return Ok((STATE_BAD, "未找到分享码".into())); }
    if pwd.is_empty() { return Ok((STATE_LOCKED, "115 需要提取码".into())); }
    let url = format!("https://115cdn.com/webapi/share/snap?share_code={}&offset=0&limit=20&receive_code={}&cid=",
        code, pwd);
    let referer = format!("https://115cdn.com/s/{}?password={}&", code, pwd);
    let h = hdr(&[
        ("priority", "u=1, i"),
        ("referer", &referer),
        ("x-requested-with", "XMLHttpRequest"),
    ]);
    let (_, body, _) = client.request_text(reqwest::Method::GET, &url, Some(h), None).await?;
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let state = json_field(&v, &["state"]).and_then(|x| x.as_bool()).unwrap_or(false);
    let errno = json_field(&v, &["errno"]).and_then(|x| x.as_i64()).unwrap_or(-1);
    let error = as_str(json_field(&v, &["error"]));
    if state && errno == 0 {
        let list_len = v.get("list").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
        let count = json_field(&v, &["count"]).and_then(|x| x.as_i64()).unwrap_or(0);
        let snap_id = as_str(json_field(&v, &["shareinfo", "snap_id"]));
        let title = as_str(json_field(&v, &["shareinfo", "share_title"]));
        let share_state = json_field(&v, &["shareinfo", "share_state"]).and_then(|x| x.as_i64()).unwrap_or(0);
        if list_len > 0 || count > 0 || !snap_id.is_empty() || !title.is_empty() || share_state == 1 {
            return Ok((STATE_OK, "分享有效".into()));
        }
        let forbid = as_str(json_field(&v, &["shareinfo", "forbid_reason"]));
        if contains_any(forbid, &["密码", "提取码"]) { return Ok((STATE_LOCKED, "需要提取码".into())); }
        return Ok((STATE_BAD, "分享不可用".into()));
    }
    if contains_any(error, &["密码", "提取码", "receive_code"]) { return Ok((STATE_LOCKED, "需要提取码".into())); }
    if contains_any(error, &["参数错误", "不存在", "失效", "share_code", "forbid", "违规", "删除", "取消"]) {
        return Ok((STATE_BAD, "分享已失效".into()));
    }
    if error.is_empty() { return Ok((STATE_UNCERTAIN, "无法确认分享状态".into())); }
    Ok((STATE_BAD, error.to_string()))
}

fn extract_115(url: &str) -> (String, String) {
    let code = url.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
    let mut pwd = url::Url::parse(url).ok()
        .and_then(|u| u.query_pairs().into_owned().find(|(k, _)| k == "password").map(|(_, v)| v))
        .unwrap_or_default();
    if pwd.is_empty() {
        let re = regex::Regex::new(r"password=([^&/]+)").unwrap();
        pwd = re.captures(url).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()).unwrap_or_default();
    }
    (code, pwd)
}

// ============ 中国移动云盘 ============
async fn check_mobile(client: &HttpClient, normalized: &str, item_pwd: &str) -> anyhow::Result<(&'static str, String)> {
    let share_id = extract_mobile(normalized);
    if share_id.is_empty() { return Ok((STATE_BAD, "未找到分享ID".into())); }
    let payload = json!({
        "getOutLinkInfoReq": {
            "account": "",
            "linkID": share_id,
            "passwd": item_pwd,
            "caSrt": 1,
            "coSrt": 1,
            "srtDr": 0,
            "bNum": 1,
            "pCaID": "root",
            "eNum": 200
        },
        "commonAccountInfo": {"account": "", "accountType": 1}
    }).to_string();
    let encrypted = encrypt_mobile(&payload)?;
    let h = hdr(&[
        ("accept", "application/json, text/plain, */*"),
        ("content-type", "application/json"),
        ("hcy-cool-flag", "1"),
        ("x-deviceinfo", "||3|12.27.0|chrome|131.0.0.0|5c7c68368f048245e1ce47f1c0f8f2d0||windows 10|1536X695|zh-CN|||"),
    ]);
    let body = json!(encrypted).to_string();
    let (_, resp, _) = client.request_text(reqwest::Method::POST,
        "https://share-kd-njs.yun.139.com/yun-share/richlifeApp/devapp/IOutLink/getOutLinkInfoV6",
        Some(h), Some(body)).await?;
    let decrypted = decrypt_mobile(resp.trim())?;
    let v: Value = serde_json::from_str(&decrypted).unwrap_or(Value::Null);
    let result_code = as_str(json_field(&v, &["resultCode"]));
    let desc = as_str(json_field(&v, &["desc"]));
    if result_code == "0" && v.get("data").is_some() && !v.get("data").unwrap().is_null() {
        return Ok((STATE_OK, "分享有效".into()));
    }
    if contains_any(desc, &["提取码", "密码", "访问码"]) { return Ok((STATE_LOCKED, "需要提取码".into())); }
    if contains_any(desc, &["失效", "不存在", "过期", "取消"]) { return Ok((STATE_BAD, "分享已失效".into())); }
    if !desc.is_empty() { return Ok((STATE_UNCERTAIN, desc.to_string())); }
    if !result_code.is_empty() { return Ok((STATE_BAD, "分享不可用".into())); }
    Ok((STATE_UNCERTAIN, "无法确认分享状态".into()))
}

fn extract_mobile(url: &str) -> String {
    let patterns = [
        r"yun\.139\.com/shareweb/#/w/i/([^&/?#]+)",
        r"caiyun\.139\.com/w/i/([^&/?#]+)",
        r"caiyun\.139\.com/m/i\?([^&/?#]+)",
        r"caiyun\.feixin\.10086\.cn/([^&/?#]+)",
    ];
    for p in patterns {
        if let Ok(re) = regex::Regex::new(p) {
            if let Some(c) = re.captures(url) {
                return c.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            }
        }
    }
    String::new()
}

fn encrypt_mobile(plaintext: &str) -> anyhow::Result<String> {
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut iv);
    let cipher = Aes128CbcEnc::new(MOBILE_KEY.into(), (&iv).into());
    let msg = plaintext.as_bytes();
    // 缓冲区 = 明文长度 + 一个块（容纳 PKCS7 padding）
    let mut buf = vec![0u8; msg.len() + 16];
    buf[..msg.len()].copy_from_slice(msg);
    let ct = cipher.encrypt_padded_mut::<Pkcs7>(&mut buf, msg.len())
        .map_err(|e| anyhow::anyhow!("加密失败: {:?}", e))?;
    let mut combined = iv.to_vec();
    combined.extend_from_slice(ct);
    Ok(base64::encode(combined))
}

fn decrypt_mobile(cipher_text: &str) -> anyhow::Result<String> {
    let data = base64::decode(cipher_text.trim().trim_matches('"'))?;
    if data.len() < 16 || (data.len() - 16) % 16 != 0 {
        anyhow::bail!("响应长度异常");
    }
    let iv = &data[..16];
    let cipher = Aes128CbcDec::new(MOBILE_KEY.into(), iv.into());
    let mut buf = data[16..].to_vec();
    let pt = cipher.decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow::anyhow!("解密失败: {:?}", e))?;
    Ok(String::from_utf8_lossy(pt).to_string())
}

// ============ 工具函数 ============
fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
