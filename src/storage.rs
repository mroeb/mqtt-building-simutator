use crate::blueprints::BlueprintStore;
use std::path::Path;
use tokio::fs;

const STORAGE_FILE: &str = "data/blueprints.json";

pub async fn load_store() -> BlueprintStore {
    if !Path::new(STORAGE_FILE).exists() {
        return BlueprintStore::default();
    }

    let Ok(content) = fs::read_to_string(STORAGE_FILE).await else {
        return BlueprintStore::default();
    };

    serde_json::from_str(&content).unwrap_or_default()
}

pub async fn save_store(store: &BlueprintStore) -> Result<(), String> {
    fs::create_dir_all("data")
        .await
        .map_err(|error| error.to_string())?;

    let content = serde_json::to_string_pretty(store).map_err(|error| error.to_string())?;

    fs::write(STORAGE_FILE, content)
        .await
        .map_err(|error| error.to_string())?;

    Ok(())
}
