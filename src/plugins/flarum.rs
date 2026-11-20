//! Flarum 论坛类插件通用实现（pan666 / bixin / quark4k / susu 等）

use crate::model::SearchResult;
use crate::plugin::{ExtMap, SearchPlugin};
use crate::plugins::common::*;
use crate::util::http::browser_headers;
use crate::util::HttpClient;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::header::HeaderValue;

pub struct FlarumPlugin {
    pub name: &'static str,
    pub base_url: &'static str,
    pub referer: &'static str,
    pub priority: i32,
    pub skip_filter: bool,
}

#[async_trait]
impl SearchPlugin for FlarumPlugin {
    fn name(&self) -> &'static str { self.name }
    fn priority(&self) -> i32 { self.priority }
    fn skip_service_filter(&self) -> bool { self.skip_filter }

    async fn search(&self, client: &HttpClient, keyword: &str, _ext: &ExtMap) -> Result<Vec<SearchResult>> {
        let mut all = Vec::new();
        // 并发抓 2 页
        let mut futs = Vec::new();
        for page in 0..2 {
            let offset = page * 50;
            let url = format!(
                "{}?include=mostRelevantPost&filter%5Bq%5D={}&page%5Boffset%5D={}&page%5Blimit%5D=50",
                self.base_url,
                crate::util::text::url_encode(keyword),
                offset
            );
            let mut headers = browser_headers(Some(self.referer));
            headers.insert("accept", HeaderValue::from_static("application/json, text/plain, */*"));
            headers.insert("x-forwarded-for", HeaderValue::from_str(&random_ip()).unwrap_or_else(|_| HeaderValue::from_static("1.1.1.1")));
            let client = client.clone();
            futs.push(async move { client.get_text(&url, Some(headers)).await });
        }
        let responses = futures::future::join_all(futs).await;

        for body in responses.into_iter().flatten() {
            match serde_json::from_str::<FlarumResponse>(&body) {
                Ok(resp) => all.extend(self.parse(resp, keyword)),
                Err(e) => tracing::debug!("[{}] JSON 解析失败: {}", self.name, e),
            }
        }
        Ok(finalize(all, keyword))
    }
}

impl FlarumPlugin {
    fn parse(&self, resp: FlarumResponse, keyword: &str) -> Vec<SearchResult> {
        let post_map = flarum_post_map(&resp.included);
        let mut results = Vec::new();
        for d in resp.data {
            if d.attributes.title.is_empty() { continue; }
            if !crate::util::text::matches_keywords(&d.attributes.title, "", keyword) { continue; }
            let post_id = d.relationships.most_relevant_post.as_ref()
                .and_then(|r| r.data.as_ref()).map(|p| p.id.clone())
                .or_else(|| d.relationships.first_post.as_ref().and_then(|r| r.data.as_ref()).map(|p| p.id.clone()))
                .unwrap_or_default();
            let content = post_map.get(&post_id).cloned().unwrap_or_default();
            let links = links_from_text(&content);
            if links.is_empty() { continue; }
            results.push(SearchResult {
                unique_id: format!("{}-{}", self.name, d.id),
                title: d.attributes.title.clone(),
                content: clean_html(&content),
                datetime: parse_datetime(d.attributes.created()),
                links,
                channel: String::new(),
                ..Default::default()
            });
        }
        results
    }
}

fn random_ip() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    format!("{}.{}.{}.{}", rng.gen_range(1..223), rng.gen_range(0..255), rng.gen_range(0..255), rng.gen_range(1..254))
}
