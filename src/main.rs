mod model;
mod config;
mod util;
mod service;
mod plugin;
mod plugins;
mod api;
mod speedtest;

use std::sync::Arc;

#[tokio::main]
async fn main() {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info".into()))
        .init();

    let cfg = config::init();

    // 注册插件
    let plugin_list = plugins::all();
    let mut plugin_count = plugin_list.len();
    let manager = Arc::new(plugin::PluginManager::new().register_all(plugin_list));
    plugin_count = manager.plugin_count();
    let _ = plugin_count;

    // 测速（可选）：只补测新增目标，筛选 TOP_N
    let channels = cfg.default_channels.clone();
    if cfg.speedtest_enabled {
        println!("[测速] 测速已启用，开始测速（仅补测新增目标）...");
        let mut report = speedtest::run_and_persist_channels(
            &cfg.speedtest_result_path,
            &channels,
            cfg.speedtest_timeout,
        ).await;

        // 插件测速：对所有已启用插件串行测速
        let plugins = manager.plugins();
        let mut prs = Vec::new();
        for p in &plugins {
            if report.plugins.contains_key(p.name()) { continue; }
            println!("[测速] 插件 {} 测速中...", p.name());
            let r = speedtest::test_plugin(p.as_ref(), cfg.speedtest_timeout).await;
            prs.push(r);
        }
        if !prs.is_empty() {
            report = speedtest::merge_plugin_results(&cfg.speedtest_result_path, prs);
        }

        if cfg.speedtest_top_n > 0 {
            let top_channels = speedtest::select_top_channels(&report, &channels, cfg.speedtest_top_n);
            let all_names = manager.names();
            let top_plugins = speedtest::select_top_plugin_names(&report, &all_names, cfg.speedtest_top_n);
            println!("[测速] 筛选: {} 个频道, {} 个插件", top_channels.len(), top_plugins.len());
            config::set_runtime_channels(top_channels);
            config::set_runtime_plugins(top_plugins);
        }
    }

    // 构建服务与路由
    let search = Arc::new(service::SearchService::new(manager.clone()));
    let check = Arc::new(service::CheckService::new());
    let state = api::AppState { search, check };
    let app = api::build_router(state);

    let addr = format!("0.0.0.0:{}", cfg.port);
    println!("pansou-rs 启动: http://{}", addr);
    println!("  插件数: {}  频道数: {}  认证: {}", plugin_count, channels.len(), cfg.auth_enabled);

    let listener = tokio::net::TcpListener::bind(&addr).await
        .expect(&format!("绑定端口 {} 失败", addr));
    axum::serve(listener, app).await.expect("服务器运行失败");
}
