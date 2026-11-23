//! 插件系统：统一 trait + 异步引擎（缓存/限流/超时/合并）+ 全局注册表

pub mod engine;
pub mod registry;

pub use engine::{ExtMap, SearchPlugin};
pub use registry::PluginManager;

use crate::model::SearchResult;
use crate::util::HttpClient;
use anyhow::Result;

/// 插件搜索函数类型（供通用插件使用）
pub type SearchFn = dyn for<'a> Fn(
    &'a HttpClient,
    &'a str,
    &'a ExtMap,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<SearchResult>>> + Send + 'a>>
    + Send + Sync;
