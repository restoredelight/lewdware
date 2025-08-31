use std::fs;

use serde::{Deserialize, Serialize};


#[derive(Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub disabled_tags: Vec<String>,
}

pub fn load_config(path: &str) -> AppConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(AppConfig { disabled_tags: vec![] })
}
