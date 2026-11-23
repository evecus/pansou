//! 结果过滤器

use crate::model::{FilterConfig, MergedLinks, SearchResponse, SearchResult};

pub fn apply_filter(mut resp: SearchResponse, filter: &Option<FilterConfig>, result_type: &str) -> SearchResponse {
    let f = match filter {
        Some(f) if !f.include.is_empty() || !f.exclude.is_empty() => f,
        _ => return resp,
    };
    let include: Vec<String> = f.include.iter().map(|s| s.to_lowercase()).collect();
    let exclude: Vec<String> = f.exclude.iter().map(|s| s.to_lowercase()).collect();

    match result_type {
        "merged_by_type" | "" => {
            if let Some(merged) = resp.merged_by_type.take() {
                let filtered = filter_merged(merged, &include, &exclude);
                resp.total = filtered.values().map(|v| v.len()).sum();
                resp.merged_by_type = Some(filtered);
            }
        }
        "all" | "results" => {
            if let Some(results) = resp.results.take() {
                resp.results = Some(filter_results(results, &include, &exclude));
                resp.total = resp.results.as_ref().map(|r| r.len()).unwrap_or(0);
                if result_type == "all" {
                    if let Some(merged) = resp.merged_by_type.take() {
                        resp.merged_by_type = Some(filter_merged(merged, &include, &exclude));
                    }
                }
            }
        }
        _ => {}
    }
    resp
}

fn filter_merged(merged: MergedLinks, include: &[String], exclude: &[String]) -> MergedLinks {
    let mut out = MergedLinks::new();
    for (k, links) in merged {
        let filtered: Vec<_> = links.into_iter().filter(|l| match_filter(&l.note, include, exclude)).collect();
        if !filtered.is_empty() { out.insert(k, filtered); }
    }
    out
}

fn filter_results(results: Vec<SearchResult>, include: &[String], exclude: &[String]) -> Vec<SearchResult> {
    results.into_iter().filter_map(|mut r| {
        if !match_filter(&r.title, include, exclude) { return None; }
        let filtered: Vec<_> = r.links.into_iter().filter(|l| {
            let wt = l.work_title.as_deref().unwrap_or("");
            let text = if wt.is_empty() { r.title.as_str() } else { wt };
            match_filter(text, include, exclude)
        }).collect();
        if filtered.is_empty() { return None; }
        r.links = filtered;
        Some(r)
    }).collect()
}

fn match_filter(text: &str, include: &[String], exclude: &[String]) -> bool {
    let lower = text.to_lowercase();
    if exclude.iter().any(|kw| lower.contains(kw)) { return false; }
    if !include.is_empty() && !include.iter().any(|kw| lower.contains(kw)) { return false; }
    true
}
