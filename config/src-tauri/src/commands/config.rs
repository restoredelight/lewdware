use crate::dto::ConfigDto;
use crate::modes::{apply_config_dto, save_to_disk};
use crate::state::State;

#[tauri::command]
pub fn get_config(state: State<'_>) -> ConfigDto {
    state.config.lock().unwrap().clone().into()
}

#[tauri::command]
pub fn save_config(state: State<'_>, config: ConfigDto) -> Result<(), String> {
    let mut current = state.config.lock().unwrap();
    let new_config = apply_config_dto(&current, config);

    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&new_config, &uploaded).map_err(|e| e.to_string())?;
    *current = new_config;
    Ok(())
}
