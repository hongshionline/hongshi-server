use axum::{Json, Router, extract::State};
use rand::Rng;
use serde::{Deserialize, Serialize};
use regex;
use crate::send_email as hongshi_send_email;
use crate::init;
use crate::init::Config;
use std::collections::HashMap;
use moka::future::Cache;
use std::sync::Arc;

//请求体
#[derive(Deserialize)]
pub struct SendVerifyEmailRequestJson{
    pub email: String,
    pub captcha_token: String,
    pub captcha_answer: String,
}

//返回体
#[derive(Deserialize, Serialize)]
pub struct SendVerifyEmailResponseJson{
    pub status: bool,
    pub token: i32,
    validity_period: i32, //单位:s
}

//返回体默认值
impl Default for SendVerifyEmailResponseJson {
    fn default() -> Self {
        Self{
            status: true,
            token: -1,
            validity_period: 300,
        }
    }
}

//AppState
#[derive(Clone)]
pub struct AppState{
    pub config: init::SendEmailConfig,
    pub verify_code_cache: Cache<i32, i32>,
}

//接口处理函数
//改send_verify_email函数的接口和定义太麻烦 不如直接包一层处理函数
pub async fn send_verify(State(app_state): State<AppState>, Json(payload): Json<SendVerifyEmailRequestJson>) -> Json<SendVerifyEmailResponseJson> {
    let code = match send_verify_email(&payload.email, app_state.config.email_address.clone(), &app_state.config.secret_id, &app_state.config.secret_key).await {
        Ok(code) => code,
        Err(error) => {
            return Json(SendVerifyEmailResponseJson{
                status: false,
                ..Default::default()
            })
        }
    };

    //生成token
    let token = rand::thread_rng().gen_range(10000..999999); //5~6位随机数token
    //追加到缓存中
    app_state.verify_code_cache.insert(token, code).await;

    Json(SendVerifyEmailResponseJson{
        token,
        ..Default::default()
    })
}

//发送邮件函数
async fn send_verify_email(email: &String, email_address: String, secret_id: &String, secret_key: &String) -> Result<i32, String>{ //return verify code
    //生成随机验证码
    let random_verify_code = rand::thread_rng().gen_range(10000..99999);

    //构建发送请求
    let send_verify_code_request = hongshi_send_email::SendEmailRequest{
        from_email_address: email_address,
        destination: vec![email.clone()],
        subject: "邮箱验证码".to_string(),
        template_id: 62793,
        template_data: serde_json::json!({ "code": random_verify_code }).to_string(),
        reply_to_addresses: None,
        trigger_type: None,
    };

    match hongshi_send_email::send_email(
        &secret_id,
        &secret_key,
        &hongshi_send_email::SEND_LOCATION,
        &send_verify_code_request
    ).await{
        Ok(_) => {},
        Err(e) => return Err(format!("Failed to send email: {}", e))
    };

    Ok(random_verify_code)
}