use std::path::PathBuf;
use serde::Deserialize;

#[derive(Default)]
pub struct Home{
    //房间列表不入数据库
    pub uuid: String,
    pub owner_uuid: String,
    pub headcount: i32,
    pub max_headcount: i32,
    pub members: Vec<String>, //成员uuid列表
    pub create_data: u64,
    pub modpack_name: String,
    pub modpack_path: PathBuf,
}