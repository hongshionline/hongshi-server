use std::fmt::format;
use std::fs;
use std::process::exit;
use std::time::Duration;
use axum::{routing, Router};
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
    let captcha_router: Router = Router::new()
        .route("/verify", routing::get(web::sign::captcha::create_captcha_image)).with_state(Arc::new(config.captcha));

    //邮箱验证码
    let email_state = web::sign::send_email::AppState{
        config: config.send_email,
        // Cache 内部是 Arc 直接clone
        verify_code_cache: verify_code_cache.clone(),
    };
    let send_email_router: Router = Router::new()
        .route("/verify/email", routing::post(web::sign::send_email::send_verify)).with_state(email_state);
    
    //注册路由
    let create_user_state = web::sign::create::AppState{
        pool,
        verify_code_cache: verify_code_cache.clone(),
    };
    let create_user_route: Router = Router::new()
        .route("/user/create", routing::post(web::sign::create::create_user)).with_state(create_user_state);

    //主路由
    let master_router: Router = Router::new().nest("/api", create_user_route
        .nest("/api",send_email_router)
        .merge(captcha_router)
    );

    tracing::info!("starting server listen : {}", config.web.listen_port);

    //启动服务
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", config.web.listen_port)).await.unwrap();
    axum::serve(listener, master_router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap(); //方便获取ip
}