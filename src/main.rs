mod model;
mod config;
mod util;
mod service;
mod plugin;
mod plugins;
mod api;

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

    // 构建服务与路由
    let search = Arc::new(service::SearchService::new(manager.clone()));
    let check = Arc::new(service::CheckService::new());
    let state = api::AppState { search, check };
    let app = api::build_router(state);

    let addr = format!("0.0.0.0:{}", cfg.port);
    println!("pansou-rs 启动: http://{}", addr);
    println!("  插件数: {}  频道数: {}  认证: {}", plugin_count, cfg.default_channels.len(), cfg.auth_enabled);

    let listener = tokio::net::TcpListener::bind(&addr).await
        .expect(&format!("绑定端口 {} 失败", addr));
    axum::serve(listener, app).await.expect("服务器运行失败");
}
