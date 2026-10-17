use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    pub output_dir: Option<String>,
}
impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .filter(|v| v.len() <= 16384)
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or("Invalid settings path")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let stage = path.with_extension("json.new");
        std::fs::write(&stage, serde_json::to_vec(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        std::fs::rename(stage, path).map_err(|e| e.to_string())
    }
}
