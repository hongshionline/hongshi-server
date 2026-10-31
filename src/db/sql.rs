use std::fmt::format;
use serde::Deserialize;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::MySqlPool;

pub const DB_NAME: &str = "hongshionline";

pub async fn init_db(address: &String, user: &String, password: &String, db_name: &str) -> Result<MySqlPool, String>{
    //初始化数据库
    let options = MySqlConnectOptions::new().password(&password).username(user).database(db_name);

    //初始化连接词
    let pool = match MySqlPoolOptions::new().max_connections(16).connect_with(options).await{
        Ok(p) => p,
        Err(e) => return Err(sqlx::Error::from(e).to_string()),
    };

    /*#[cfg(debug_assertions)]
    {
        //清库
        sqlx::query("TRUNCATE TABLE ?").bind(&db_name).execute(&pool).await.unwrap();
    }*/

    //执行初始化语句
    match sqlx::migrate!("./migrations/").run(&pool).await{
        Ok(p) => Ok(pool),
        Err(e) => Err(sqlx::Error::from(e).to_string()),
    }
}