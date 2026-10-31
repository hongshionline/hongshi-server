//! 按 IP 的每日请求限额。
//!
//! 目标：把"每天每 IP 最多 N 次"封装成两行代码，调用方不需要接触
//! `Key` / `StorageKey` / `FromRef` 这些概念。
//!
//! # 用法
//!
//! ```ignore
//! use axum::{middleware, routing::post, Router};
//! use hongshi_server::daily_limit::{daily_limit, DailyLimiter};
//!
//! // 每个 IP 每天最多 20 次
//! let limiter = DailyLimiter::new("user_create", 20);
//!
//! let app = Router::new()
//!     .route("/user/create", post(create_user))
//!     .with_state(create_user_state)
//!     .layer(middleware::from_fn_with_state(limiter, daily_limit));
//! ```
//!
//! 也可以在 handler 里手动检查：
//!
//! ```ignore
//! match limiter.check(ip).await {
//!     DailyLimit::Allowed { remaining } => { /* 放行 */ }
//!     DailyLimit::Exceeded { retry_after_secs } => { /* 返回 429 */ }
//! }
//! ```
//!
//! # 行为说明
//!
//! - 计数窗口按**北京时间自然日**（UTC+8，每天 0 点重置）；改 [`DAY_OFFSET_SECS`] 可换时区。
//! - 每次请求按 `(限流器名字, IP, 日期)` 计数；同名限流器共享额度，不同名字互不影响。
//! - 超限时中间件自动返回 **429**，JSON body（`status/info/message/retry_after/limit`），
//!   并带 `Retry-After`、`X-RateLimit-Limit`、`X-RateLimit-Remaining` 响应头。
//! - 计数在进程内存里（moka），重启清零；多实例部署时各实例独立计数，
//!   如果需要跨实例共享，要换成 Redis 等外部存储。
//! - 客户端 IP 来自 `ConnectInfo<SocketAddr>`，因此服务启动必须使用
//!   `axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())`。
//!   如果请求里拿不到 IP，会记录一条 warning 并**跳过限流**（方便单测/本地调试）。
//! - 如果服务部署在反向代理后面，`ConnectInfo` 拿到的是代理 IP，
//!   需要自己从 `X-Forwarded-For` 取真实 IP（只信任你自己的代理写入的头）。

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use moka::future::Cache;
use serde::Serialize;

/// 一天有多少秒。
const DAY_SECS: u64 = 86_400;
/// 自然日按东八区（北京时间）计算；要改成 UTC 就设为 0。
const DAY_OFFSET_SECS: u64 = 8 * 3600;
/// 缓存条目的存活时间：48 小时，保证跨天后旧 key 会被自动清理。
const CACHE_TTL_SECS: u64 = 48 * 3600;
/// 最多同时跟踪多少个 key（IP×天×限流器），防止被大量 IP 刷爆内存。
const MAX_TRACKED_KEYS: u64 = 100_000;

/// 一天内某个 IP 的检查结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DailyLimit {
    /// 放行；`remaining` 是本次请求后当天还剩多少次。
    Allowed { remaining: u32 },
    /// 超限；`retry_after_secs` 是距离次日 0 点（额度重置）还有多少秒。
    Exceeded { retry_after_secs: u64 },
}

/// 按 IP 的每日限额计数器。
///
/// 内部是一个 moka 缓存（`(name, ip, day) -> 原子计数`），`Clone` 是浅拷贝，
/// 克隆出来的多个句柄共享同一份计数，因此可以放进 axum 的 state 里跨请求复用。
#[derive(Clone)]
pub struct DailyLimiter {
    name: Arc<str>,
    limit: u32,
    cache: Cache<String, Arc<AtomicU32>>,
}

impl DailyLimiter {
    /// 创建一个每日限流器。
    ///
    /// - `name`：限流器名字，会拼进缓存 key；不同名字的额度互相独立。
    /// - `limit`：每个 IP 每天的请求上限（必须大于 0）。
    ///
    /// # Panics
    ///
    /// `limit == 0` 时会 panic（属于配置错误）。
    pub fn new(name: impl Into<String>, limit: u32) -> Self {
        assert!(limit > 0, "每日限流上限必须大于 0");

        Self {
            name: Arc::from(name.into()),
            limit,
            cache: Cache::builder()
                .max_capacity(MAX_TRACKED_KEYS)
                .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
                .build(),
        }
    }

    /// 记录一次请求，并判断是否超过当日限额。
    ///
    /// 只有真正"通过"的调用才会消耗额度；超限后的请求也会继续计数，
    /// 但返回值始终是 [`DailyLimit::Exceeded`]，次日 0 点后自动恢复。
    pub async fn check(&self, ip: IpAddr) -> DailyLimit {
        let now = unix_secs();
        let day = (now + DAY_OFFSET_SECS) / DAY_SECS;
        let key = format!("{}:{}:{}", self.name, ip, day);

        let counter = self
            .cache
            .entry(key)
            .or_insert_with(async { Arc::new(AtomicU32::new(0)) })
            .await
            .into_value();

        let used = counter.fetch_add(1, Ordering::Relaxed);
        if used < self.limit {
            DailyLimit::Allowed {
                remaining: self.limit - used - 1,
            }
        } else {
            let next_day_start = (day + 1) * DAY_SECS - DAY_OFFSET_SECS;
            DailyLimit::Exceeded {
                retry_after_secs: next_day_start.saturating_sub(now),
            }
        }
    }

    /// 清空某个 IP 当天的计数（测试或后台解锁用）。
    pub async fn reset(&self, ip: IpAddr) {
        let now = unix_secs();
        let day = (now + DAY_OFFSET_SECS) / DAY_SECS;
        let key = format!("{}:{}:{}", self.name, ip, day);
        self.cache.invalidate(&key).await;
    }

    /// 每个 IP 每天的上限。
    pub fn limit(&self) -> u32 {
        self.limit
    }

    /// 限流器名字。
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// 每日限额中间件。
///
/// 通过 `middleware::from_fn_with_state(limiter, daily_limit)` 挂到路由上；
/// 中间件自己携带 `DailyLimiter`，不需要动路由原来的 state 类型。
pub async fn daily_limit(
    State(limiter): State<DailyLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let Some(ip) = client_ip(&req) else {
        tracing::warn!("daily_limit: 请求缺少 ConnectInfo<SocketAddr>，跳过限流");
        return next.run(req).await;
    };

    match limiter.check(ip).await {
        DailyLimit::Allowed { remaining } => {
            let mut response = next.run(req).await;
            let headers = response.headers_mut();
            headers.insert(
                HeaderName::from_static("x-ratelimit-limit"),
                header_value(limiter.limit()),
            );
            headers.insert(
                HeaderName::from_static("x-ratelimit-remaining"),
                header_value(remaining),
            );
            response
        }
        DailyLimit::Exceeded { retry_after_secs } => {
            tracing::warn!(
                ip = %ip,
                path = %req.uri().path(),
                limit = limiter.limit(),
                "触发每日请求限额"
            );
            too_many_requests(limiter.limit(), retry_after_secs)
        }
    }
}

fn client_ip(req: &Request) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip())
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn header_value(value: u32) -> HeaderValue {
    HeaderValue::from_str(&value.to_string()).expect("数字一定是合法的 header 值")
}

#[derive(Serialize)]
struct DailyLimitErrorBody {
    status: bool,
    info: &'static str,
    message: &'static str,
    retry_after: u64,
    limit: u32,
}

fn too_many_requests(limit: u32, retry_after_secs: u64) -> Response {
    let body = DailyLimitErrorBody {
        status: false,
        info: "E429",
        message: "daily request limit exceeded",
        retry_after: retry_after_secs,
        limit,
    };

    let mut response = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();

    let headers = response.headers_mut();
    headers.insert(
        header::RETRY_AFTER,
        HeaderValue::from_str(&retry_after_secs.to_string())
            .expect("数字一定是合法的 header 值"),
    );
    headers.insert(
        HeaderName::from_static("x-ratelimit-limit"),
        header_value(limit),
    );
    headers.insert(
        HeaderName::from_static("x-ratelimit-remaining"),
        HeaderValue::from_static("0"),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, last))
    }

    #[tokio::test]
    async fn allows_requests_up_to_limit_then_denies() {
        let limiter = DailyLimiter::new("test", 2);

        assert_eq!(
            limiter.check(ip(1)).await,
            DailyLimit::Allowed { remaining: 1 }
        );
        assert_eq!(
            limiter.check(ip(1)).await,
            DailyLimit::Allowed { remaining: 0 }
        );
        assert!(matches!(
            limiter.check(ip(1)).await,
            DailyLimit::Exceeded { retry_after_secs } if retry_after_secs > 0
        ));
    }

    #[tokio::test]
    async fn different_ips_have_independent_quota() {
        let limiter = DailyLimiter::new("test", 1);

        assert!(matches!(
            limiter.check(ip(1)).await,
            DailyLimit::Allowed { .. }
        ));
        assert!(matches!(
            limiter.check(ip(1)).await,
            DailyLimit::Exceeded { .. }
        ));
        // 另一个 IP 不受影响
        assert!(matches!(
            limiter.check(ip(2)).await,
            DailyLimit::Allowed { .. }
        ));
    }

    #[tokio::test]
    async fn different_limiters_are_independent() {
        let a = DailyLimiter::new("a", 1);
        let b = DailyLimiter::new("b", 1);

        assert!(matches!(
            a.check(ip(1)).await,
            DailyLimit::Allowed { .. }
        ));
        assert!(matches!(
            a.check(ip(1)).await,
            DailyLimit::Exceeded { .. }
        ));
        // 另一个限流器对同一 IP 仍可放行
        assert!(matches!(
            b.check(ip(1)).await,
            DailyLimit::Allowed { .. }
        ));
    }

    #[tokio::test]
    async fn reset_gives_back_quota() {
        let limiter = DailyLimiter::new("test", 1);

        assert!(matches!(
            limiter.check(ip(1)).await,
            DailyLimit::Allowed { .. }
        ));
        assert!(matches!(
            limiter.check(ip(1)).await,
            DailyLimit::Exceeded { .. }
        ));

        limiter.reset(ip(1)).await;

        assert!(matches!(
            limiter.check(ip(1)).await,
            DailyLimit::Allowed { .. }
        ));
    }

    #[test]
    fn limit_and_name_are_accessible() {
        let limiter = DailyLimiter::new("demo", 5);
        assert_eq!(limiter.limit(), 5);
        assert_eq!(limiter.name(), "demo");
    }
}
