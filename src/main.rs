use std::fmt::format;
use std::fs;
use std::process::exit;
use std::time::Duration;
use axum::{routing, Router, extract::DefaultBodyLimit};
//use moka::future::CacheBuilder;
use serde::Deserialize;
use db::sql;
use hongshi_server::db;
use hongshi_server::init;
use moka::future as moka;
use hongshi_server::web;
use std::sync::Arc;
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};
use tracing_appender::non_blocking;
use tracing_subscriber::{fmt, prelude::*, EnvFilter, registry};
use std::net::SocketAddr;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use axum::middleware;
use hongshi_server::daily_limit::{daily_limit, DailyLimiter};

#[tokio::main]
async fn main() {
    //读取配置
    let config = match init::init_config(){Ok(c) => c, Err(err) => { eprint!("read config failed: {}", err); exit(1) } };

    //初始化日志系统
    let file_appender = tracing_appender::rolling::daily("logs/", "web");
    let (non_blocking, _guard) = non_blocking(file_appender);
    let registry_ = tracing_subscriber::registry()
        .with(EnvFilter::new("trace"))
        .with(fmt::layer().with_ansi(false).with_writer(non_blocking))
        .with(fmt::layer().with_ansi(true)) //输出到控制台 可选
        .init();
    tracing::info!("starting up\n logging init successfully");

    //初始化数据库
    let pool = match sql::init_db(&config.sql.address, &config.sql.user, &config.sql.password, sql::DB_NAME).await{
        Ok(pool) => pool,
        Err(err) => {
            eprint!("init db failed: {}", err);
            exit(1);
        }
    };
    tracing::info!("init database successfully");

    //创建verify code缓存
    let verify_code_cache = moka::Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(300))
        .build();

    //创建路由

    //图形验证码
    //限流: 5s/次 突发请求最大10个
    let governor_conf_captcha = GovernorConfigBuilder::default()
        .per_second(5)
        .burst_size(10)
        .finish()
        .unwrap();

    let captcha_router: Router = Router::new()
        .route("/verify", routing::get(web::sign::captcha::create_captcha_image)).with_state(Arc::new(config.captcha)).layer(GovernorLayer::new(governor_conf_captcha));

    //邮箱验证码
    //限流: 1min/次 突发请求最大5个
    let governor_conf_email = GovernorConfigBuilder::default()
        .per_second(60)
        .burst_size(5)
        .finish()
        .unwrap();

    let email_state = web::sign::send_email::AppState{
        config: config.send_email,
        // Cache 内部是 Arc 直接clone
        verify_code_cache: verify_code_cache.clone(),
    };
    let send_email_router: Router = Router::new()
        .route("/verify/email", routing::post(web::sign::send_email::send_verify))
        .with_state(email_state)
        .layer(GovernorLayer::new(governor_conf_email))
        .layer(middleware::from_fn_with_state(DailyLimiter::new("send_email", 3), daily_limit));//每天限流 3次/天
    
    //注册用户路由
    //限流: 10h/次 突发请求最大5个
    let governor_conf_create_user = GovernorConfigBuilder::default()
        .per_second(3_6000)
        .burst_size(5)
        .finish()
        .unwrap();

    let create_user_state = web::sign::create::AppState{
        pool,
        verify_code_cache: verify_code_cache.clone(),
    };
    let create_user_route: Router = Router::new()
        .route("/user/create", routing::post(web::sign::create::create_user))
        .with_state(create_user_state)
        .layer(GovernorLayer::new(governor_conf_create_user))
        .layer(middleware::from_fn_with_state(DailyLimiter::new("create_user", 2), daily_limit)); //每天限流 2次/天;

    //主路由
    let master_router: Router = Router::new().nest("/api", create_user_route
        .nest("/api",send_email_router)
        .merge(captcha_router)
    ).layer(DefaultBodyLimit::max(config.web.max_request_size as usize)); //请求体大小限制

    tracing::info!("starting server listen : {}", config.web.listen_port);

    //启动服务
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", config.web.listen_port)).await.unwrap();
    axum::serve(listener, master_router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap(); //方便获取ip
}