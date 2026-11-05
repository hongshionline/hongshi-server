
use std::collections::HashMap;
use std::result;
use std::sync::{Arc, RwLock};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use crate::home::Home;
use axum::{Json, extract::State};
use crate::auth;
//use crate::web::home::push_modpack::PushModpackResponse;

//请求体
#[derive(Deserialize)]
pub struct DeleteHomeRequestJson{
    pub uuid: String,
}

//返回体
#[derive(Serialize)]
pub struct DeleteHomeResponseJson{
    pub status: bool,
    pub info: String
}

#[derive(Clone)]
pub struct AppState{
    pub pool: sqlx::Pool<sqlx::MySql>,
    pub homes: Arc<RwLock<HashMap<String, Home>>>,
}

//请求处理函数
pub async fn delete_home(State(app_state): State<AppState>, header: HeaderMap, Json(payload): Json<DeleteHomeRequestJson>) -> Json<DeleteHomeResponseJson>{
    //鉴权
    let apikey = match auth::get_apikey(&header).await {
        Some(apikey) => apikey,
        None => return Json(DeleteHomeResponseJson{status: false, info: "E400".to_string()})
    };
    if auth::auth_apikey_str(&apikey, &app_state.pool).await == false {
        return Json(DeleteHomeResponseJson{status: false, info: "E404".to_string()});
    };
    //获取房间所属者
    let owner = match app_state.homes.read() {
        Ok(owner) => match owner.get(&payload.uuid) {
            Some(owner) => owner.owner_uuid.clone(),
            None => return Json(DeleteHomeResponseJson{status: false, info: "E404-no home".to_string()})
        },
        Err(_) => return Json(DeleteHomeResponseJson{status: false, info: "E500".to_string()})
    };
    //获取上传者uuid
    let pusher = match auth::get_info_from_apikey(&apikey, &app_state.pool, "uuid").await {
        Some(pusher) => pusher,
        None => return Json(DeleteHomeResponseJson{status: false, info: "E404".to_string()})
    };
    //效验是否相同
    if pusher != owner {return Json(DeleteHomeResponseJson{status: false, info: "E403".to_string()})};

    //释放owner
    drop(owner);

    //释放房间
    let result_ = delete_home_in_db_and_remove_modpack(app_state.homes.clone(), app_state.pool.clone(), &payload.uuid).await;

    if result_ {
        //拿到homes写权限
        let mut home_write = match app_state.homes.write() {
            Ok(home) => home,
            Err(_e) => {
                tracing::error!("can not recycle home lock");
                return Json(DeleteHomeResponseJson { status: false, info: "E500".to_string() });
            }
        };
        //在内存中释放房间
        home_write.remove(&payload.uuid);
        
    } else {return Json(DeleteHomeResponseJson{status: false, info: "E404".to_string()});}

    Json(DeleteHomeResponseJson{status: true, info: "200".to_string()})
}

pub async fn delete_home_in_db_and_remove_modpack(homes: Arc<RwLock<HashMap<String, Home>>>, pool: sqlx::Pool<sqlx::MySql>, home_uuid: &String) -> bool{
    //删除房间在数据库中 并移除modpack
    //删除内存中的房间需要write 所以手动删除
    let home_read = match homes.read() {
        Ok(home) => home,
        Err(_e) => {
            tracing::error!("can not delete home: {}", home_uuid);
            return false
        }
    };
    //获取房间信息
    let home_info = match home_read.get(home_uuid) {
        Some(h) => h,
        None => {
            tracing::error!("can not delete home: {}", home_uuid);
            return false
        }
    };

    //在数据库中删除
    match sqlx::query("UPDATE `user` SET `have_home` = NULL WHERE `have_home` = ?;")
        .bind(home_uuid)
        .execute(&pool)
        .await{
        Ok(result) => {
            if result.rows_affected() == 0 {
                tracing::warn!(home_uuid = home_uuid, "no user holds this room");
            }
        },
        Err(e) => tracing::error!(home_uuid = home_uuid, error = %e, "can not clear user.have_home"),
    };

    //删除整合包缓存文件
    if !home_info.modpack_path.as_os_str().is_empty() {
        match tokio::fs::remove_file(&home_info.modpack_path).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::error!(
                    path = ?home_info.modpack_path,
                    error = %e,
                    "can not remove modpack file"
                ),
        }
    };

    true
}