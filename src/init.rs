use std::fs;
use serde::Deserialize;
use serde_json;

//定义配置文件结构体
#[derive(Deserialize)]
pub struct Config{
    pub sql: SqlConfig,
    pub send_email: SendEmailConfig,
    pub captcha: CaptchaConfig,
    pub web: WebConfig,
    pub modpack: ModpackConfig,
}

#[derive(Deserialize)]
pub struct SqlConfig{
    pub address: String,
    pub user: String,
    pub password: String,
}

#[derive(Deserialize, Clone)]
pub struct SendEmailConfig{
    pub secret_id: String,
    pub secret_key: String,
    pub login_url: String,
    pub email_address: String,
}

#[derive(Deserialize)]
pub struct CaptchaConfig{
    pub secret: String,
}

#[derive(Deserialize)]
pub struct WebConfig{
    pub listen_port: u32,
    pub max_request_size: u32,
}

#[derive(Deserialize, Clone)]
pub struct ModpackConfig{
    pub cache_path: String,
}

pub fn init_config() -> Result<Config, String> {
    //读取配置文件
    let config = fs::read_to_string("config.json");
    let config: Config = match serde_json::from_str(&config.unwrap()){
        Ok(config) => config,
        Err(e) => return Err(e.to_string())
    };
    Ok(config)
}