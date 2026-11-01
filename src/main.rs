use std::collections::HashMap;
//use std::fmt::format;
//use std::fs;
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
use std::sync::{Arc, RwLock};
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};
use tracing_appender::non_blocking;
use tracing_subscriber::{fmt, prelude::*, EnvFilter, registry};
use std::net::SocketAddr;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use axum::middleware;
use tracing_subscriber::fmt::layer;
use hongshi_server::auth::auth_me;
use hongshi_server::daily_limit::{daily_limit, DailyLimiter};
use hongshi_server::home;

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

    //每次启动先清空user的have_home字段
    if let Err(err) = sqlx::query("UPDATE `user` SET `have_home` = NULL")
        .execute(&pool)
        .await
    {
        tracing::error!("reset user.have_home failed: {err}");
    }

    //创建verify code缓存
    let verify_code_cache = moka::Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(300))
        .build();

    //全局房间列表
    let homes: Arc<RwLock<HashMap<String, home::Home>>> = Arc::new(RwLock::new(HashMap::new()));

    //创建路由

    //图形验证码
    //限流: 5s/次 突发请求最大10个
    let governor_conf_captcha = GovernorConfigBuilder::default()
        .per_second(5)
        .burst_size(10)
        .finish()
        .unwrap();

    let captcha_router: Router = Router::new()
        .route("/verify", routing::get(web::sign::captcha::create_captcha_image))
        .with_state(Arc::new(config.captcha))
        .layer(GovernorLayer::new(governor_conf_captcha));

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
        pool: pool.clone(),
        verify_code_cache: verify_code_cache.clone(),
    };
    let create_user_route: Router = Router::new()
        .route("/user/create", routing::post(web::sign::create::create_user))
        .with_state(create_user_state)
        .layer(GovernorLayer::new(governor_conf_create_user))
        .layer(middleware::from_fn_with_state(DailyLimiter::new("create_user", 2), daily_limit)); //每天限流 2次/天;

    //测试apikey是否有效
    //限流 2s/次
    let governor_conf_auth_me = GovernorConfigBuilder::default()
        .per_second(2)
        .burst_size(1)
        .finish()
        .unwrap();

    let auth_me_route: Router = Router::new()
        .route("/auth", routing::get(auth_me))
        .with_state(pool.clone())
        .layer(GovernorLayer::new(governor_conf_auth_me));
    
    //创建房间接口
    //限流1min/次
    let governor_conf_create_home = GovernorConfigBuilder::default()
        .per_second(60)
        .burst_size(1)
        .finish()
        .unwrap();

    let create_home_state = web::home::create::AppState{
        pool: pool.clone(),
        homes: homes.clone(),
    };
    let create_home_route: Router = Router::new()
        .route("/home/create", routing::post(web::home::create::create_home))
        .with_state(create_home_state)
        .layer(GovernorLayer::new(governor_conf_create_home));

    //上传整合包并关联房间
    //限流 1min/次
    let governor_conf_push_modpack = GovernorConfigBuilder::default()
        .per_second(60)
        .burst_size(1)
        .finish()
        .unwrap();

    let push_modpack_state = web::home::push_modpack::AppState{
        config: config.modpack,
        homes: homes.clone(),
        pool: pool.clone(),
    };
    let push_modpack_route: Router = Router::new()
        .route("/home/modpack", routing::post(web::home::push_modpack::pust_modpack_to_home))
        .with_state(push_modpack_state)
        .layer(GovernorLayer::new(governor_conf_push_modpack))
        .layer(DefaultBodyLimit::max((20 * 1024 * 1024) as usize)); //单独设置请求体可以达到20mb

    //主路由
    let master_router: Router = Router::new().nest("/api", create_user_route
        .merge(send_email_router)
        .merge(captcha_router)
        .merge(auth_me_route)
        .merge(create_home_route)
        .merge(push_modpack_route)
    ).layer(DefaultBodyLimit::max(config.web.max_request_size as usize)); //请求体大小限制

    tracing::info!("starting server listen : {}", config.web.listen_port);

    //启动服务
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", config.web.listen_port)).await.unwrap();
    axum::serve(listener, master_router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap(); //方便获取ip
}