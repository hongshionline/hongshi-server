use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use uuid;
use crate::apikey;

//apikey算法
pub async fn build_apikey() -> String {
    //生成核心随机字符串
    let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let mut secret: String = String::with_capacity(64);
    for i in 0..128 {
        secret.push(*(chars.choose(&mut rng).unwrap()) as char);
    };
    //生成最终apikey
    format!("hs-{}-{}", uuid::Uuid::new_v4().to_string(), secret)
}