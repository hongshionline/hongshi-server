use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::Arc;
use axum::{Router,body::Bytes, Json};
use axum::extract::State;
use crate::home::Home;
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use serde::{Deserialize, Serialize};
use crate::{auth, init};
use std::path::Path;
use axum::http::HeaderMap;
use tokio::io::AsyncWriteExt;
use crate::auth::auth_apikey_str;

//请求体
#[derive(TryFromMultipart)]
pub struct PushModpackRequest{
    pub uuid: String,
    pub modpack: FieldData<Bytes>
}

//返回体
#[derive(Serialize)]
pub struct PushModpackResponse {
    pub status: bool,
    pub info: String,
}

//默认值
impl Default for PushModpackResponse {
    fn default() -> Self {
        Self{
            status: true,
            info: "".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct AppState{
    pub config: init::ModpackConfig,
    pub homes: Arc<RwLock<HashMap<String, Home>>>,
    pub pool: sqlx::Pool<sqlx::MySql>
}

//请求处理函数
pub async fn pust_modpack_to_home(State(app_state): State<AppState>, header: HeaderMap, TypedMultipart(payload): TypedMultipart<PushModpackRequest>) -> Json<PushModpackResponse> {
    //鉴权
    let apikey = match auth::get_apikey(&header).await {
        Some(apikey) => apikey,
        None => return Json(PushModpackResponse{status: false, info: "E400".to_string()})
    };
    if auth::auth_apikey_str(&apikey, &app_state.pool).await == false {
        return Json(PushModpackResponse{status: false, info: "E404".to_string()});
    };
    //获取房间所属者
    let owner = match app_state.homes.read() {
        Ok(owner) => match owner.get(&payload.uuid) {
            Some(owner) => owner.owner_uuid.clone(),
            None => return Json(PushModpackResponse{status: false, info: "E404-no home".to_string()})
        },
        Err(_) => return Json(PushModpackResponse{status: false, info: "E500".to_string()})
    };
    //获取上传者uuid
    let pusher = match auth::get_info_from_apikey(&apikey, &app_state.pool, "uuid").await {
        Some(pusher) => pusher,
        None => return Json(PushModpackResponse{status: false, info: "E404".to_string()})
    };
    //效验是否相同
    if pusher != owner {return Json(PushModpackResponse{status: false, info: "E403".to_string()})};

    //释放owner
    drop(owner);

    //提取文件名和生成安全文件路径
    let file_name = match payload.modpack.metadata.file_name.clone(){ Some(file_name) => file_name, None => return Json(PushModpackResponse{status: false, info: "E400".to_string()})};
    let safe_file_name = format!("{}.mrpack", uuid::Uuid::new_v4().to_string());
    let safe_file_path = Path::new(&app_state.config.cache_path).join(Path::new(&safe_file_name));
    //写入文件 最大20mb 可以直接写入无需流式
    let mut file = match tokio::fs::File::create(&safe_file_path).await { Ok(f) => f, Err(_e) => return Json(PushModpackResponse{status: false, info:"E500".to_string()})};
    match file.write_all(&payload.modpack.contents).await {
        Ok(_) => {},
        Err(_e) => return Json(PushModpackResponse{status: false, info:"E500-write file error".to_string()})
    };
    //将文件与房间绑定起来
    //寻找对应房间
    match app_state.homes.write() {
        Ok(mut h) => {
            match h.get_mut(&payload.uuid) {
                Some(h) => {
                    h.modpack_path = safe_file_path;
                    h.modpack_name = file_name;
                    Json(PushModpackResponse::default())
                },
                None => Json(PushModpackResponse{status: false, info:"E400".to_string()})
            }
        }
        Err(_e) => Json(PushModpackResponse{status: false, info:"E404".to_string()})
    }
}