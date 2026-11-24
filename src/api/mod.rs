//! API 路由层：axum 实现 search / check / health / auth，内嵌前端 SPA。

use crate::config;
use crate::model::{CheckRequest, Response, SearchRequest};
use crate::service::{CheckService, SearchService};
use crate::util::jwt;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response as AxumResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};

#[derive(RustEmbed)]
#[folder = "frontend/"]
struct Frontend;

/// 应用共享状态
#[derive(Clone)]
pub struct AppState {
    pub search: Arc<SearchService>,
    pub check: Arc<CheckService>,
    pub plugins: Vec<Arc<dyn crate::plugin::SearchPlugin>>,
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/api/search", get(search_get).post(search_post))
        .route("/api/check/links", post(check_links))
        .route("/api/health", get(health))
        .route("/api/speedtest/channels", post(speedtest_channels))
        .route("/api/speedtest/plugins", post(speedtest_plugins))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/verify", post(auth_verify))
        .route("/api/auth/logout", post(auth_logout));

    Router::new()
        .merge(api_routes)
        .route("/", get(serve_index))
        .route("/favicon.ico", get(serve_favicon))
        .route("/assets/*path", get(serve_asset))
        .fallback(serve_index)
        .layer(cors)
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state)
}

// ============ 认证中间件 ============
async fn auth_middleware(
    State(_state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> AxumResponse {
    let cfg = config::get();
    if !cfg.auth_enabled {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    if !path.starts_with("/api") || is_public_path(&path) {
        return next.run(req).await;
    }
    let auth = req.headers().get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    const PREFIX: &str = "Bearer ";
    if !auth.starts_with(PREFIX) {
        let code = if auth.is_empty() { "AUTH_TOKEN_MISSING" } else { "AUTH_TOKEN_INVALID_FORMAT" };
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "未授权", "code": code}))).into_response();
    }
    let token = &auth[PREFIX.len()..];
    match jwt::validate_token(token, &cfg.auth_jwt_secret) {
        Ok(_) => next.run(req).await,
        Err(_) => (StatusCode::UNAUTHORIZED,
            Json(json!({"error": "未授权：令牌无效或已过期", "code": "AUTH_TOKEN_INVALID"}))).into_response(),
    }
}

fn is_public_path(path: &str) -> bool {
    path.starts_with("/api/auth/login")
        || path.starts_with("/api/auth/logout")
        || path.starts_with("/api/health")
}

// ============ 搜索 ============
async fn search_post(State(st): State<AppState>, Json(req): Json<SearchRequest>) -> AxumResponse {
    do_search(&st, req).await
}

async fn search_get(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> AxumResponse {
    let mut req = SearchRequest::default();
    req.kw = q.get("kw").cloned().unwrap_or_default();
    req.channels = split_csv(q.get("channels"));
    req.plugins = q.get("plugins").map(|s| split_csv(Some(s))).filter(|v| !v.is_empty());
    req.conc = q.get("conc").and_then(|v| v.parse().ok()).unwrap_or(0);
    req.refresh = q.get("refresh").map(|v| v == "true").unwrap_or(false);
    req.res = q.get("res").cloned().unwrap_or_default();
    req.src = q.get("src").cloned().unwrap_or_default();
    req.cloud_types = q.get("cloud_types").map(|s| split_csv(Some(s))).filter(|v| !v.is_empty());
    if let Some(ext_s) = q.get("ext") {
        if ext_s != "{}" {
            match serde_json::from_str::<serde_json::Map<String, Value>>(ext_s) {
                Ok(m) => req.ext = m,
                Err(e) => return (StatusCode::BAD_REQUEST,
                    Json(json!({"code":400,"message":format!("无效的ext参数格式: {}",e)}))).into_response(),
            }
        }
    }
    if let Some(f_s) = q.get("filter") {
        match serde_json::from_str::<crate::model::FilterConfig>(f_s) {
            Ok(f) => req.filter = Some(f),
            Err(e) => return (StatusCode::BAD_REQUEST,
                Json(json!({"code":400,"message":format!("无效的filter参数格式: {}",e)}))).into_response(),
        }
    }
    do_search(&st, req).await
}

async fn do_search(st: &AppState, mut req: SearchRequest) -> AxumResponse {
    let cfg = config::get();
    if req.channels.is_empty() { req.channels = cfg.default_channels.clone(); }
    if req.res.is_empty() || req.res == "merge" { req.res = "merged_by_type".into(); }
    if req.src.is_empty() { req.src = "all".into(); }

    // 参数互斥（与 Go 一致）
    match req.src.as_str() {
        "tg" => req.plugins = None,
        "plugin" => req.channels.clear(),
        _ => {}
    }

    match st.search.search(&req).await {
        Ok(data) => Json(Response::success(data)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR,
            Json(Response::<()>::error(500, format!("搜索失败: {}", e)))).into_response(),
    }
}

fn split_csv(v: Option<&String>) -> Vec<String> {
    v.map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

// ============ 链接检测 ============
async fn check_links(State(st): State<AppState>, Json(req): Json<CheckRequest>) -> AxumResponse {
    if req.items.is_empty() {
        return (StatusCode::BAD_REQUEST,
            Json(Response::<()>::error(400, "items不能为空"))).into_response();
    }
    let data = st.check.check(&req).await;
    Json(data).into_response()
}

// ============ 健康检查 ============
async fn health(State(st): State<AppState>) -> AxumResponse {
    let cfg = config::get();
    let mut resp = json!({
        "status": "ok",
        "auth_enabled": cfg.auth_enabled,
        "plugins_enabled": cfg.async_plugin_enabled,
        "channels": cfg.default_channels,
        "channels_count": cfg.default_channels.len(),
        "tg_reachability": crate::service::tg_reachability::snapshot(),
    });
    if cfg.async_plugin_enabled {
        let names = st.search.plugin_manager.names();
        resp["plugin_count"] = json!(names.len());
        resp["plugins"] = json!(names);
    }
    Json(resp).into_response()
}

// ============ 延迟测试 ============
/// POST /api/speedtest/channels  可选 body: {"channels": ["..."]}，省略则测全部
async fn speedtest_channels(
    State(_st): State<AppState>,
    body: Option<Json<Value>>,
) -> AxumResponse {
    let cfg = config::get();
    let channels: Vec<String> = body
        .and_then(|Json(v)| v.get("channels").and_then(|c| c.as_array()).map(|arr| {
            arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
        }))
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| cfg.default_channels.clone());
    let timeout = Duration::from_secs(crate::speedtest::TIMEOUT_SECS);
    let results = crate::speedtest::test_channels(&channels, timeout).await;
    Json(Response::success(json!({ "results": results }))).into_response()
}

/// POST /api/speedtest/plugins  可选 body: {"plugins": ["..."]}，省略则测全部
async fn speedtest_plugins(
    State(st): State<AppState>,
    body: Option<Json<Value>>,
) -> AxumResponse {
    let cfg = config::get();
    let selected: Option<Vec<String>> = body.and_then(|Json(v)| {
        v.get("plugins").and_then(|p| p.as_array()).map(|arr| {
            arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()
        })
    });
    let plugins: Vec<Arc<dyn crate::plugin::SearchPlugin>> = match selected {
        Some(names) if !names.is_empty() => st.plugins.iter()
            .filter(|p| names.iter().any(|n| n == p.name())).cloned().collect(),
        _ => st.plugins.clone(),
    };
    let timeout = Duration::from_secs(crate::speedtest::TIMEOUT_SECS);
    let results = crate::speedtest::test_plugins(&plugins, timeout).await;
    Json(Response::success(json!({ "results": results }))).into_response()
}

// ============ 认证接口 ============
async fn auth_login(Json(body): Json<Value>) -> AxumResponse {
    let cfg = config::get();
    if !cfg.auth_enabled {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "认证功能未启用"}))).into_response();
    }
    let username = body.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
    if username.is_empty() || password.is_empty() {
        return (StatusCode::BAD_REQUEST,
            Json(json!({"error": "参数错误：用户名和密码不能为空"}))).into_response();
    }
    if cfg.auth_users.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "认证系统未正确配置"}))).into_response();
    }
    match cfg.auth_users.get(username) {
        Some(p) if p == password => {
            match jwt::generate_token(username, &cfg.auth_jwt_secret, cfg.auth_token_expiry) {
                Ok(token) => {
                    let expires = chrono::Utc::now().timestamp() + cfg.auth_token_expiry.as_secs() as i64;
                    Json(json!({"token": token, "expires_at": expires, "username": username})).into_response()
                }
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "生成令牌失败"}))).into_response(),
            }
        }
        _ => (StatusCode::UNAUTHORIZED,
            Json(json!({"error": "用户名或密码错误"}))).into_response(),
    }
}

async fn auth_verify(req: Request<axum::body::Body>) -> AxumResponse {
    let cfg = config::get();
    if !cfg.auth_enabled {
        return Json(json!({"valid": true, "message": "认证功能未启用"})).into_response();
    }
    let auth = req.headers().get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    const PREFIX: &str = "Bearer ";
    let token = auth.strip_prefix(PREFIX).unwrap_or("");
    match jwt::validate_token(token, &cfg.auth_jwt_secret) {
        Ok(claims) => Json(json!({"valid": true, "username": claims.username})).into_response(),
        Err(_) => (StatusCode::UNAUTHORIZED, Json(json!({"error": "未授权"}))).into_response(),
    }
}

async fn auth_logout() -> AxumResponse {
    Json(json!({"message": "退出成功"})).into_response()
}

// ============ 前端静态资源 ============
fn serve_html(bytes: Vec<u8>) -> AxumResponse {
    let mut resp = (StatusCode::OK, bytes).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"));
    resp
}

async fn serve_index() -> AxumResponse {
    match Frontend::get("index.html") {
        Some(f) => serve_html(f.data.to_vec()),
        None => (StatusCode::NOT_FOUND, "index.html not found").into_response(),
    }
}

async fn serve_favicon() -> AxumResponse {
    match Frontend::get("favicon.ico") {
        Some(f) => {
            let mime = "image/x-icon".to_string();
            let mut resp = (StatusCode::OK, f.data.to_vec()).into_response();
            if let Ok(v) = HeaderValue::from_str(&mime) {
                resp.headers_mut().insert(header::CONTENT_TYPE, v);
            }
            resp
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_asset(axum::extract::Path(path): axum::extract::Path<String>) -> AxumResponse {
    let key = format!("assets/{}", path);
    match Frontend::get(&key) {
        Some(f) => {
            let mime = if key.ends_with(".js") { "application/javascript" }
                else if key.ends_with(".css") { "text/css" }
                else { "application/octet-stream" };
            let mut resp = (StatusCode::OK, f.data.to_vec()).into_response();
            if let Ok(v) = HeaderValue::from_str(&mime) {
                resp.headers_mut().insert(header::CONTENT_TYPE, v);
            }
            resp
        }
        None => serve_index().await,
    }
}
