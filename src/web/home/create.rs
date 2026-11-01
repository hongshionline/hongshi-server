use std::collections::HashMap;
use axum::{Json, Router};
use axum::extract::State;
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use crate::auth;
use sqlx;
use std::sync::{RwLock, Arc};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use crate::auth::get_apikey;
use crate::home;

//请求体
#[derive(Deserialize)]
pub struct CreateHomeRequestJson{
    pub game_version: String,
    pub mod_loader: String, //模组加载器较多 无法全部枚举(旧加载器)
}

//返回体
#[derive(Serialize)]
pub struct CreateHomeResponseJson{
    pub status: bool,
    pub info: String,
    pub uuid: String,
}

//默认值
impl Default for CreateHomeResponseJson{
    fn default() -> Self {
        Self{
            status: true,
            info: "".to_string(),
            uuid: "".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::Pool<sqlx::MySql>,
    pub homes: Arc<RwLock<HashMap<String, home::Home>>>,
}

//请求处理函数
pub async fn create_home(State(app_state): State<AppState>, heard: HeaderMap, Json(payload): Json<CreateHomeRequestJson>) -> Json<CreateHomeResponseJson> {
    //获取apikey
    let apikey = match auth::get_apikey(&heard).await {Some(v) => v, None => return Json(CreateHomeResponseJson { status: false, info: "E400".to_string(), ..Default::default()})};
    
    //检验apikey是否存在
    if auth::auth_apikey_str(&apikey, &app_state.pool).await == false {
        return Json(CreateHomeResponseJson { status: false, info: "E404".to_string(), ..Default::default()})
    }
    
    //获取创建者信息
    let owner_uuid = match sqlx::query_scalar("SELECT `uuid` FROM `user` WHERE `apikey` = ? LIMIT 1;")
        .bind(apikey)
        .fetch_optional(&app_state.pool)
        .await{
        Ok(owner_uuid) => match owner_uuid{
            Some(uuid) => uuid,
            None => return Json(CreateHomeResponseJson { status: false, info: "E404".to_string(), ..Default::default()})
        },
        Err(_) => return Json(CreateHomeResponseJson { status: false, info: "E404".to_string(), ..Default::default()})
    };
    
    //创建房间
    let uuid = Uuid::new_v4().to_string();

    //尝试创建房间 如果已经有了则报错
    let claim = sqlx::query(
        "UPDATE `user` SET `have_home` = ? WHERE `uuid` = ? AND `have_home` IS NULL",
    )
    .bind(&uuid)
    .bind(&owner_uuid)
    .execute(&app_state.pool)
    .await;
    match claim {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => {
            return Json(CreateHomeResponseJson {
                status: false,
                info: "E409".to_string(), // 已拥有房间
                ..Default::default()
            })
        }
        Err(_) => {
            return Json(CreateHomeResponseJson {
                status: false,
                info: "E500".to_string(),
                ..Default::default()
            })
        }
    }

    let home_ = home::Home{
        uuid: uuid.clone(),
        owner_uuid,
        headcount: 0,
        max_headcount: 10,
        members: Vec::new(),
        create_data: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        ..Default::default()
    };
    //注册房间
    let mut home_write = match app_state.homes.write() {
        Ok(write) => write,
        Err(_) => {
            //回滚数据库
            let pool = app_state.pool.clone();
            let room_uuid = uuid.clone();
            //需要单独开任务
            //或者下次统一用tokio::sync::RwLock
            tokio::spawn(async move {
                let _ = sqlx::query("UPDATE `user` SET `have_home` = NULL WHERE `have_home` = ?")
                    .bind(room_uuid)
                    .execute(&pool)
                    .await;
            });
            return Json(CreateHomeResponseJson {
                status: false,
                info: "E500".to_string(),
                uuid: "".to_string(),
            });
        }
    };
    
    home_write.insert(uuid.clone(), home_);

    //成功:
    Json(CreateHomeResponseJson{
        status: true,
        uuid,
        ..Default::default()
    })
}