use std::ptr::null;
use axum::{Json, Router, extract::State};
use moka::future::Cache;
use rand::Rng;
use serde::Deserialize;
use serde::Serialize;
use regex;
use crate::send_email as hongshi_send_email;
use crate::init;
use crate::init::Config;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::apikey as build_apikey;

//请求体
#[derive(Deserialize)]
pub struct CreateUserJson{
    pub username: String,
    pub password: String,
    pub email: String,
    pub verify_code_token: i32,
    pub verify_code: i32,
}

//返回体
#[derive(Deserialize, Serialize)]
pub struct CreateUserResponseJson{
    pub status: bool,
    pub info: String,
    pub apikey: String,
}

//设置默认值
impl Default for CreateUserResponseJson{
    fn default() -> Self{
        Self{
            status: true,
            info: "".to_string(),
            apikey: String::from("")
        }
    }
}

//状态结构体
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::Pool<sqlx::MySql>,
    //pub config: init::SendEmailConfig,
    pub verify_code_cache: Cache<i32, i32>, //moka的Cache本身就是Arc了 不需要包一层Arc
    //pub rng: rand::rngs::StdRng,
}

//请求处理函数
pub async fn create_user(State(app_state): State<AppState>, Json(payload): Json<CreateUserJson>) -> Json<CreateUserResponseJson> {
    //校验验证码
    let code = match app_state.verify_code_cache.remove(&payload.verify_code_token).await {
        Some(code) => code,
        None => return Json(CreateUserResponseJson{
            status: false,
            info: "E404".to_string(),
            ..Default::default()
        }),
    };

    //预分配空间
    let mut apikey:String = String::with_capacity(168);
    if code == payload.verify_code {
        //验证成功
        //生成apikey
        apikey = build_apikey::build_apikey().await;

        //写入数据库
        match sqlx::query("INSERT INTO user (uuid, username, password, apikey, email, last_sign_time, status, verify) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(payload.username.clone())
            .bind(payload.password.clone())
            .bind(&apikey)
            .bind(payload.email.clone())
            .bind(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs())
            .bind(0)
            .bind(1)
            .execute(&app_state.pool)
            .await{
            Ok(_) => {},
            Err(err) => return Json(CreateUserResponseJson{status: false, info: "E500".to_string(), ..Default::default()}),
        };
    }

    Json(CreateUserResponseJson{
        apikey,
        ..Default::default()
    })
}