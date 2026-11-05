use std::collections::HashMap;
use std::path::PathBuf;
use crate::home::Home;
use std::sync::RwLock;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MAX_HOME_LIVE_TIME: u64 = 60 * 60 * 5;

pub struct CollectRecycleHome{
    pub owner_uuid: String,
    pub home_uuid: String,
    pub modpack_path: PathBuf,
}

pub async fn do_once_home_recycle(homes: Arc<RwLock<HashMap<String, Home>>>, pool: sqlx::Pool<sqlx::MySql>) {
    //回收房间 10min/次
    //获取当前时间
    let now_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let mut collect_home:Vec<CollectRecycleHome> = Vec::new();
    {
        //拿到homes写权限
        let mut home_write = match homes.write() {
            Ok(home) => home,
            Err(_e) => {
                tracing::error!("can not recycle home lock");
                return
            }
        };

        //收集删除房间信息
        for (uuid, home) in home_write.iter() {
            if now_time.saturating_sub(home.create_data) > MAX_HOME_LIVE_TIME {
                collect_home.push( CollectRecycleHome{
                    owner_uuid: home.owner_uuid.clone(),
                    home_uuid: uuid.clone(),
                    modpack_path: home.modpack_path.clone()
                });
            }
        }

        //删除房间 在内存中
        for ch in &collect_home {
            home_write.remove(&ch.home_uuid);
        }
    }
    //删除房间 在数据库中
    for ch in &collect_home {
        match sqlx::query("UPDATE `user` SET `have_home` = NULL WHERE `have_home` = ?;")
            .bind(&ch.home_uuid)
            .execute(&pool)
            .await{
            Ok(result) => {
                if result.rows_affected() == 0 {
                    tracing::warn!(home_uuid = %ch.home_uuid, "no user holds this room");
                }
            },
            Err(e) => tracing::error!(home_uuid = %ch.home_uuid, error = %e, "can not clear user.have_home"),
        };

        //删除整合包缓存文件
        if !ch.modpack_path.as_os_str().is_empty() {
            match tokio::fs::remove_file(&ch.modpack_path).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::error!(
                    path = ?ch.modpack_path,
                    error = %e,
                    "can not remove modpack file"
                ),
            }
        }
    };

    //打印日志:
    tracing::info!("recycle home ok, recycle: {} homes", collect_home.len());
}

//循环清理
pub fn loop_recycle_homes(homes: Arc<RwLock<HashMap<String, Home>>>, pool: sqlx::Pool<sqlx::MySql>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60 * 10)).await;
            do_once_home_recycle(homes.clone(), pool.clone()).await;
        }
    });
}