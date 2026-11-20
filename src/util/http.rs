//! HTTP 客户端封装：支持代理（HTTP/SOCKS5）、重试、随机 UA

use once_cell::sync::Lazy;
use reqwest::{Client, ClientBuilder, Proxy, header::{HeaderMap, HeaderValue}};
use std::time::Duration;
use crate::config;

/// 常用 UA 列表（与 Go 版本一致）
pub const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/92.0.4515.107 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.1.2 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:90.0) Gecko/20100101 Firefox/90.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.114 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/92.0.4515.107 Safari/537.36",
    // 更新一点的 UA，提高通过率
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
];

pub fn random_ua() -> &'static str {
    let idx = rand::random::<usize>() % USER_AGENTS.len();
    USER_AGENTS[idx]
}

/// 默认浏览器请求头
pub fn browser_headers(referer: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(reqwest::header::USER_AGENT, HeaderValue::from_static_str_or_default(random_ua()));
    h.insert(reqwest::header::ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8"));
    h.insert(reqwest::header::ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"));
    h.insert(reqwest::header::CONNECTION, HeaderValue::from_static("keep-alive"));
    if let Some(r) = referer {
        if let Ok(v) = HeaderValue::from_str(r) { h.insert(reqwest::header::REFERER, v); }
    }
    h
}

trait FromStaticStrOrDefault {
    fn from_static_str_or_default(s: &str) -> HeaderValue;
}
impl FromStaticStrOrDefault for HeaderValue {
    fn from_static_str_or_default(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).unwrap_or_else(|_| HeaderValue::from_static("Mozilla/5.0"))
    }
}

/// 构建 reqwest Client（可指定代理）
pub fn build_client(proxy: Option<&str>, timeout: Duration) -> Client {
    let mut b = ClientBuilder::new()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(20)
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .danger_accept_invalid_certs(true);

    if let Some(p) = proxy.filter(|s| !s.trim().is_empty()) {
        match Proxy::all(p.trim()) {
            Ok(px) => { b = b.proxy(px); }
            Err(e) => tracing::warn!("代理地址无效 {}: {}", p, e),
        }
    }
    b.build().expect("构建 HTTP 客户端失败")
}

/// 全局默认 HTTP 客户端
pub static GLOBAL_CLIENT: Lazy<Client> = Lazy::new(|| {
    let cfg = config::get();
    let proxy = if !cfg.proxy_url.is_empty() {
        Some(cfg.proxy_url.as_str())
    } else if !cfg.http_proxy_url.is_empty() {
        Some(cfg.http_proxy_url.as_str())
    } else if !cfg.https_proxy_url.is_empty() {
        Some(cfg.https_proxy_url.as_str())
    } else { None };
    build_client(proxy, Duration::from_secs(60))
});

/// 带重试的 GET/POST 请求封装
#[derive(Clone)]
pub struct HttpClient {
    pub client: Client,
    pub retries: usize,
}

impl HttpClient {
    pub fn new(client: Client, retries: usize) -> Self { Self { client, retries } }

    pub fn global() -> Self {
        Self::new(GLOBAL_CLIENT.clone(), 2)
    }

    /// 带重试的文本 GET 请求
    pub async fn get_text(&self, url: &str, headers: Option<HeaderMap>) -> anyhow::Result<String> {
        let mut last_err = anyhow::anyhow!("请求失败");
        for i in 0..=self.retries {
            let mut req = self.client.get(url);
            if let Some(ref h) = headers { req = req.headers(h.clone()); }
            else { req = req.headers(browser_headers(None)); }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        return Ok(body);
                    }
                    last_err = anyhow::anyhow!("HTTP {}", status);
                }
                Err(e) => last_err = anyhow::Error::from(e),
            }
            if i < self.retries {
                tokio::time::sleep(Duration::from_millis(300 * (i as u64 + 1))).await;
            }
        }
        Err(last_err)
    }

    /// 发起一次带完整响应的请求（返回状态码 + 文本 body + 响应头），不做状态判断、不重试。
    /// 检测服务需要读取原始状态码（如 123pan 的 403 视为有效）时使用。
    pub async fn request_text(
        &self,
        method: reqwest::Method,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<String>,
    ) -> anyhow::Result<(u16, String, HeaderMap)> {
        let mut req = self.client.request(method, url);
        if let Some(ref h) = headers { req = req.headers(h.clone()); }
        else { req = req.headers(browser_headers(None)); }
        if let Some(b) = body { req = req.body(b); }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let resp_headers = resp.headers().clone();
        let text = resp.text().await.unwrap_or_default();
        Ok((status, text, resp_headers))
    }

    /// 带重试的二进制 GET 请求
    pub async fn get_bytes(&self, url: &str, headers: Option<HeaderMap>) -> anyhow::Result<Vec<u8>> {
        let mut last_err = anyhow::anyhow!("请求失败");
        for i in 0..=self.retries {
            let mut req = self.client.get(url);
            if let Some(ref h) = headers { req = req.headers(h.clone()); }
            else { req = req.headers(browser_headers(None)); }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    match resp.bytes().await {
                        Ok(b) if status.is_success() => return Ok(b.to_vec()),
                        Ok(_) => last_err = anyhow::anyhow!("HTTP {}", status),
                        Err(e) => last_err = anyhow::Error::from(e),
                    }
                }
                Err(e) => last_err = anyhow::Error::from(e),
            }
            if i < self.retries {
                tokio::time::sleep(Duration::from_millis(300 * (i as u64 + 1))).await;
            }
        }
        Err(last_err)
    }
}

/// 便捷：构建带超时的插件客户端（每插件可自定义代理）
pub fn plugin_client(proxy: Option<&str>, timeout: Duration) -> HttpClient {
    HttpClient::new(build_client(proxy, timeout), 2)
}
