use axum::extract::State;
use axum::http::HeaderMap;
use sqlx::MySqlPool;
use axum::Json;
use serde::Serialize;
use uuid::Uuid;
use crate::web::home::create::CreateHomeResponseJson;

//查询apikey是否存在
pub async fn auth_apikey(header: &HeaderMap, pool: &MySqlPool) -> bool {
    //获取apikey
    let apikey = match get_apikey(&header).await{
        Some(v)=>v,
        None=>return false
    };

    //校验apikey
    match sqlx::query_scalar::<_, String>(
        "SELECT `uuid` FROM `user` WHERE `apikey` = ? LIMIT 1",
    )
    .bind(apikey)
    .fetch_optional(pool)
    .await
    {
        Ok(value) =>
            match value {
                Some(row) => true,
                None => false,
            },
        Err(_) => false,
    }
}

//查询apikey是否存在 使用string 适合已经获取apikey的情况
pub async fn auth_apikey_str(apikey: &String, pool: &MySqlPool) -> bool {
    //校验apikey
    match sqlx::query_scalar::<_, String>(
        "SELECT `uuid` FROM `user` WHERE `apikey` = ? LIMIT 1",
    )
        .bind(apikey)
        .fetch_optional(pool)
        .await
    {
        Ok(value) =>
            match value {
                Some(row) => true,
                None => false,
            },
        Err(_) => false,
    }
}

//尝试获取apikey
pub async fn get_apikey(header: &HeaderMap) -> Option<String> {
    //提取apikey
    match header.get("apikey") {
        Some(value) => match value.to_str() {
            Ok(value) => Some(value.to_string()),
            Err(_) => None,
        },
        None => None,
    }
}

//尝试使用apikey获取用户信息
//内部方法 绝不能暴露给外部
pub async fn get_info_from_apikey(apikey: &String, pool: &MySqlPool, info: &str) -> Option<String> {
    match sqlx::query_scalar(format!("SELECT `{info}` FROM `user` WHERE `apikey` = ? LIMIT 1;").as_str()) //info可以为任何参数 所以绝对只能在内部使用
        .bind(apikey)
        .fetch_optional(pool)
        .await {
        Ok(row) => match row {
            Some(row) => Some(row),
            None => None,
        },
        Err(_) => None,
    }
}

//小接口:校验apikey是否有效
//返回体
#[derive(Serialize)]
pub struct AuthResponseJson {
    status: bool,
}

//接口处理函数
pub async fn auth_me(State(pool): State<sqlx::Pool<sqlx::MySql>>, heard: HeaderMap) -> Json<AuthResponseJson> {
    //校验apikey
    if auth_apikey(&heard, &pool).await {
        return Json(AuthResponseJson { status: true });
    } else {
        return Json(AuthResponseJson { status: false });
    }
}