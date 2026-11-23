//! TG 阶段可达性门（对齐 Go 原版 service/tg_reachability.go）
//!
//! 背景（Go 原版实测）：t.me 在被墙的网络里不是"连接被拒绝"，而是"连接被静默丢包"，
//! 于是 111 个频道请求全部挂满超时（整阶段稳定 4 秒），而这 4 秒换来的结果恒为 0 条。
//! 前端第一次 src=tg / src=all 调用因此被拖成最慢路径，用户看到的就是空等或空结果。
//!
//! 三条硬约束，缺一不可：
//!  1. 探测只在后台跑，搜索路径只读状态。结论过期时也只读旧结论（fail-open），绝不阻塞。
//!  2. 门开（不可达）时跳过 TG 阶段：不写缓存、不计频道失败，让插件结果正常返回。
//!  3. 拿不准一律按"可达"处理；只有连续 N 次探测失败才敢关门，避免误杀慢但可用的 TG。
//!
//! 判定依据是"能否拿到 HTTP 响应"而非状态码：404/403 同样证明网络通，
//! 只有传输层失败（DNS/连接/TLS/超时）才算不可达。

use crate::config;
use crate::util::http::GLOBAL_CLIENT;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 单次探测总预算：明显小于频道请求超时才有意义。探测在后台跑，不影响请求时延。
const PROBE_BUDGET: Duration = Duration::from_secs(3);
/// 可达结论保鲜期：可达时没必要频繁探测。
const REACHABLE_TTL: Duration = Duration::from_secs(5 * 60);
/// 不可达结论保鲜期：故意取短，t.me 恢复后一分钟内自动回到正常路径。
const UNREACHABLE_TTL: Duration = Duration::from_secs(60);
/// 后台重探检查间隔。
const PROBE_CHECK_INTERVAL: Duration = Duration::from_secs(3);
/// 连续失败多少次才敢关门（fail-open：中途保持上一结论）。
const REQUIRED_FAILURES: i32 = 2;
/// 探测 UA。
const PROBE_UA: &str = "Mozilla/5.0 (compatible; PanSou-TGProbe)";

static REACHABLE: AtomicBool = AtomicBool::new(true); // fail-open：默认可达
static CHECKED_AT: AtomicI64 = AtomicI64::new(0); // Unix 毫秒
static FAIL_STREAK: AtomicI32 = AtomicI32::new(0);
static REASON: Mutex<String> = Mutex::new(String::new());
/// 单飞标记：防止后台循环与手动触发并发探测同一件事。
/// 用 AtomicBool 而非 std Mutex——后者 guard 非 Send，不能在 spawn 的 async 里跨 await 持有。
static PROBING: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

/// RAII：探测结束（含提前 return）自动释放单飞标记。
struct ProbeGuard;
impl Drop for ProbeGuard {
    fn drop(&mut self) { PROBING.store(false, Ordering::SeqCst); }
}

fn set_reason(s: String) {
    if let Ok(mut g) = REASON.lock() { *g = s; }
}
fn get_reason() -> String {
    REASON.lock().map(|g| g.clone()).unwrap_or_default()
}

/// 启动后台探测循环（重复调用无副作用）。
pub fn start_probe() {
    if STARTED.swap(true, Ordering::SeqCst) { return; }
    let reason = "尚未探测（按可达处理）".to_string();
    set_reason(reason);
    tracing::info!(
        "[TG可达性] 后台探测已启动：可达保鲜 {:?}、不可达保鲜 {:?}；搜索路径只读结论不阻塞",
        REACHABLE_TTL, UNREACHABLE_TTL
    );
    tokio::spawn(async move {
        probe_once().await;
        let mut tick = tokio::time::interval(PROBE_CHECK_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            // 正在确认失败（连击未达阈值）时尽快复检；否则按保鲜期决定是否重探。
            let streak = FAIL_STREAK.load(Ordering::SeqCst);
            if streak > 0 && streak < REQUIRED_FAILURES {
                probe_once().await;
                continue;
            }
            if fresh() { continue; }
            probe_once().await;
        }
    });
}

/// 搜索路径调用：当前是否值得执行 TG 阶段。
pub fn reachable() -> bool {
    REACHABLE.load(Ordering::SeqCst)
}

/// /api/health 暴露的快照。
pub fn snapshot() -> serde_json::Value {
    let checked_ms = CHECKED_AT.load(Ordering::SeqCst);
    let checked_ago = if checked_ms > 0 {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
        serde_json::json!((now_ms - checked_ms) / 1000)
    } else {
        serde_json::Value::Null
    };
    serde_json::json!({
        "reachable": REACHABLE.load(Ordering::SeqCst),
        "reason": get_reason(),
        "fail_streak": FAIL_STREAK.load(Ordering::SeqCst),
        "required_failures": REQUIRED_FAILURES,
        "checked_seconds_ago": checked_ago,
    })
}

fn fresh() -> bool {
    let at = CHECKED_AT.load(Ordering::SeqCst);
    if at == 0 { return false; }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let ttl = if !REACHABLE.load(Ordering::SeqCst) || FAIL_STREAK.load(Ordering::SeqCst) > 0 {
        UNREACHABLE_TTL
    } else {
        REACHABLE_TTL
    };
    (now_ms - at) < ttl.as_millis() as i64
}

/// 探测目标：配置里的第一个频道页。用真实频道页而非首页，覆盖真正要走的路径。
fn probe_url() -> String {
    let ch = config::get().default_channels.first().cloned().unwrap_or_default();
    if ch.is_empty() { "https://t.me/".to_string() } else { format!("https://t.me/s/{}", ch) }
}

async fn probe_once() {
    // 单飞：已有探测在跑就跳过本次。
    if PROBING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    let _guard = ProbeGuard;
    let started = Instant::now();
    let url = probe_url();
    // 用带代理的全局客户端（对齐 Go util.GetHTTPClient）；外层 3s timeout 控制预算，
    // 被墙时静默丢包的连接会被 timeout 掐断，不会挂满。
    let result = tokio::time::timeout(PROBE_BUDGET, async {
        GLOBAL_CLIENT
            .head(&url)
            .header(reqwest::header::USER_AGENT, PROBE_UA)
            .send()
            .await
    }).await;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    CHECKED_AT.store(now_ms, Ordering::SeqCst);

    let (ok, reason) = match result {
        Ok(Ok(resp)) => {
            // 拿到 HTTP 响应即视为可达（含 404/403/405），关闭 body 便于连接复用。
            let code = resp.status().as_u16();
            (true, format!("探测 {} 返回 HTTP {} ({:?})", url, code, started.elapsed()))
        }
        Ok(Err(e)) => (false, format!("探测 {} 失败: {}", url, e)),
        Err(_) => (false, format!("探测 {} 超时({:?})", url, PROBE_BUDGET)),
    };

    if ok {
        FAIL_STREAK.store(0, Ordering::SeqCst);
        REACHABLE.store(true, Ordering::SeqCst);
        set_reason(reason.clone());
        log_transition(true, &reason);
        return;
    }

    // 失败：先记连击，未达阈值保持上一结论（fail-open），不因单次超时关掉 TG。
    let streak = FAIL_STREAK.fetch_add(1, Ordering::SeqCst) + 1;
    if streak < REQUIRED_FAILURES {
        set_reason(format!("{}（连续失败 {}/{}，暂不改判）", reason, streak, REQUIRED_FAILURES));
        tracing::warn!("[TG可达性] 探测失败 {}/{}，暂不改判，{:?} 后复检：{}",
            streak, REQUIRED_FAILURES, PROBE_CHECK_INTERVAL, reason);
        return;
    }
    REACHABLE.store(false, Ordering::SeqCst);
    let confirmed = format!("连续 {} 次探测失败：{}", streak, reason);
    set_reason(confirmed.clone());
    log_transition(false, &confirmed);
}

static LAST_LOGGED_OK: AtomicBool = AtomicBool::new(true);
fn log_transition(ok: bool, reason: &str) {
    if LAST_LOGGED_OK.load(Ordering::SeqCst) == ok { return; }
    LAST_LOGGED_OK.store(ok, Ordering::SeqCst);
    if ok {
        tracing::info!("[TG可达性] 已恢复可达：{}", reason);
    } else {
        tracing::warn!("[TG可达性] 转为不可达，TG 阶段将跳过（不写缓存、不计频道失败）：{}", reason);
    }
}

/// 供测试/手动即时判定：立即探测一次并返回结论。
pub async fn probe_now() -> bool {
    probe_once().await;
    reachable()
}
