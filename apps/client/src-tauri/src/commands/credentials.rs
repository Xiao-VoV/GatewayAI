use crate::platform::credentials::{self, StoredCredentials};

#[tauri::command]
pub fn save_credentials(payload: StoredCredentials) -> Result<(), String> {
    credentials::save(payload)
}

#[tauri::command]
pub fn load_credentials() -> Result<Option<StoredCredentials>, String> {
    credentials::load()
}

#[tauri::command]
pub fn clear_credentials() -> Result<(), String> {
    credentials::clear()
}
