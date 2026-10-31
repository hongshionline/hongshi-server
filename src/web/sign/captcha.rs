use axum::{extract::State, Json};
use captcha_rs::CaptchaBuilder;
use serde::Serialize;
use crate::init;
use std::sync::Arc;

//返回体
#[derive(Serialize)]
pub struct CreateCaptchaResponseJson {
    pub status: bool,
    pub token: String,
    pub image: String, // base64 data URI
}

//默认值
impl Default for CreateCaptchaResponseJson {
    fn default() -> Self {
        Self {
            status: true,
            token: "".to_string(),
            image: "".to_string(),
        }
    }
}

//接口处理函数
pub async fn create_captcha_image(State(config): State<Arc<init::CaptchaConfig>> ) -> Json<CreateCaptchaResponseJson> {
    //生成验证码
    let captcha = CaptchaBuilder::new()
        .length(4)
        .width(150)
        .height(60)
        .complexity(3)
        .compression(75)
        .build();

    let (image, token) = match captcha.as_tuple(&config.secret, 300) {
        Some((image, token)) => (image, token),
        None => {
            //token 生成失败
            return Json(CreateCaptchaResponseJson {
                status: false,
                ..Default::default()
            });
        }
    };

    //tracing::info!("get image successfully");

    Json(CreateCaptchaResponseJson {
        status: true,
        image,
        token,
    })
}
