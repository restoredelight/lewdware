use std::path::PathBuf;

use anyhow::Result;
use dioxus::prelude::*;
use shared::{
    mode::read_mode_metadata_async,
    user_config::{AppConfig, AppConfigStoreExt, Mode},
};

use crate::pack::MediaPack;

#[derive(Clone, Copy)]
pub struct Modes {
    config: Store<AppConfig>,
    pub default_mode: Signal<shared::mode::Metadata>,
    pub pack_modes: Signal<Vec<PackMode>>,
    pub uploaded_modes: Signal<Vec<UploadedMode>>,
}

impl Modes {
    pub fn new(config: Store<AppConfig>, default_mode: shared::mode::Metadata, uploaded_modes: Vec<UploadedMode>) -> Self {
        let mut result = Self {
            config,
            default_mode: Signal::new(default_mode),
            pack_modes: Signal::new(Vec::new()),
            uploaded_modes: Signal::new(uploaded_modes),
        };

        result.update_options();

        result
    }

    async fn update_pack_inner(&mut self, pack: &Option<MediaPack>) -> Result<()> {
        if let Some(pack) = pack {
            let modes = pack
                .get_modes()
                .await
                .inspect_err(|_| self.pack_modes.write().clear())?;

            *self.pack_modes.write() = modes;
        } else {
            self.pack_modes.write().clear();
        }

        Ok(())
    }

    pub async fn update_pack(&mut self, pack: &Option<MediaPack>) -> Result<()> {
        let result = self.update_pack_inner(pack).await;

        let mut updated_selected_mode = false;

        for mode_file in self.pack_modes.read().iter() {
            if let Some((name, _)) = mode_file.metadata.modes.first() {
                self.config.mode().set(Mode::Pack {
                    id: mode_file.id,
                    mode: name.clone(),
                });
                updated_selected_mode = true;
                break;
            }
        }

        if !updated_selected_mode {
            self.config.mode().set(Mode::default());
        }

        result
    }

    pub fn set_mode(&mut self, mode: shared::user_config::Mode) {
        self.config.mode().set(mode);

        self.update_options();
    }

    fn update_options(&mut self) {
        if let Some(mode) = self.selected_mode() {
            let config_mode = self.config.mode().read().clone();
            let mut mode_options = self.config.mode_options();
            let mut mode_options_w = mode_options.write();
            let options = mode_options_w.entry(config_mode).or_default();

            for (key, option) in mode.options.iter() {
                let value = options.entry(key.clone()).or_insert_with(|| option.default_value());

                if !option.matches_value(value) {
                    options.insert(key.clone(), option.default_value());
                }
            }

            options.retain(|key, _| mode.options.contains_key(key));
        }
    }

    pub async fn upload_mode(&mut self, path: PathBuf) -> Result<()> {
        if self.config.uploaded_modes().read().contains(&path) {
            return Ok(());
        }

        let mut file = tokio::fs::File::open(&path).await?;

        let (_, metadata) = read_mode_metadata_async(&mut file).await?;

        self.uploaded_modes
            .write()
            .push(UploadedMode { path: path.clone(), metadata });

        self.config.uploaded_modes().push(path);

        Ok(())
    }

    pub fn remove_mode(&mut self, path: &PathBuf) {
        self.config.uploaded_modes().retain(|uploaded| uploaded != path);
        self.uploaded_modes.retain(|mode| &mode.path != path);
    }

    pub fn get_mode(&self, mode: &shared::user_config::Mode) -> Option<shared::mode::Mode> {
        match mode {
            Mode::Default(mode) => self.default_mode.read().modes.get(mode).cloned(),
            Mode::Pack { id, mode } => self
                .pack_modes
                .read()
                .iter()
                .find(|mode_file| mode_file.id == *id)
                .and_then(|mode_file| mode_file.metadata.modes.get(mode).cloned()),
            Mode::File { path, mode } => self
                .uploaded_modes
                .read()
                .iter()
                .find(|mode_file| &mode_file.path == path)
                .and_then(|mode_file| mode_file.metadata.modes.get(mode).cloned()),
        }
    }

    pub fn selected_mode(&self) -> Option<shared::mode::Mode> {
        self.get_mode(&*self.config.mode().read())
    }
}

pub struct PackMode {
    pub id: u64,
    pub metadata: shared::mode::Metadata,
}

#[derive(Clone)]
pub struct UploadedMode {
    pub path: PathBuf,
    pub metadata: shared::mode::Metadata,
}
