use axum::{Json, Router};

pub struct Member{
    pub user_uuid: String,
}

pub struct ModJson{
    pub name: String,
    pub is_modrinth_mod: bool,
    pub version: String,
    pub download_address: String,
    pub total_count: i32,
}

pub struct Home{
    pub address: String,
    pub members: Vec<Member>,
    pub mods: Json<ModJson>
}

pub fn create_home(){
    
}