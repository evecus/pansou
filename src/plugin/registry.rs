//! 插件注册表与管理器

use crate::plugin::engine::{AsyncEngine, SearchPlugin};
use crate::config;
use std::sync::Arc;

pub struct PluginManager {
    /// (插件实例, 异步引擎)
    entries: Vec<(Arc<dyn SearchPlugin>, AsyncEngine)>,
}

impl PluginManager {
    pub fn new() -> Self { Self { entries: Vec::new() } }

    /// 按配置过滤注册全局插件
    pub fn register_all(mut self, plugins: Vec<Arc<dyn SearchPlugin>>) -> Self {
        let cfg = config::get();
        for p in plugins {
            let enabled = match &cfg.enabled_plugins {
                None => true,                       // 未设置：启用全部
                Some(list) => list.iter().any(|n| n == p.name()),
            };
            if enabled {
                let engine = AsyncEngine::new(p.clone());
                self.entries.push((p, engine));
            }
        }
        self
    }

    pub fn plugin_count(&self) -> usize { self.entries.len() }

    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|(p, _)| p.name().to_string()).collect()
    }

    pub fn engines(&self) -> Vec<&AsyncEngine> {
        self.entries.iter().map(|(_, e)| e).collect()
    }

    /// 按名称筛选插件（用于 src=plugin 且指定 plugins 参数）
    pub fn filter_by_names(&self, names: &[String]) -> Vec<&AsyncEngine> {
        self.entries.iter()
            .filter(|(p, _)| names.iter().any(|n| n == p.name()))
            .map(|(_, e)| e)
            .collect()
    }

    /// 按名字找引擎
    pub fn get(&self, name: &str) -> Option<&AsyncEngine> {
        self.entries.iter().find(|(p, _)| p.name() == name).map(|(_, e)| e)
    }

    /// 所有插件实例
    pub fn plugins(&self) -> Vec<Arc<dyn SearchPlugin>> {
        self.entries.iter().map(|(p, _)| p.clone()).collect()
    }
}
