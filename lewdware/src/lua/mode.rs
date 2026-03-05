use std::{
    cell::RefCell,
    collections::HashMap,
    io::{Read, Seek},
};

use anyhow::bail;
use shared::mode::{Metadata, SourceFile, read_mode_metadata, read_source_file};

pub trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

pub struct Mode {
    file: RefCell<Box<dyn ReadSeek>>,
    files: HashMap<String, SourceFile>,
    cache: RefCell<HashMap<String, mlua::Value>>,
    loading: RefCell<HashMap<String, tokio::sync::broadcast::WeakSender<()>>>,
}

impl Mode {
    pub fn new(file: Box<dyn ReadSeek>, files: HashMap<String, SourceFile>) -> Self {
        Self {
            file: RefCell::new(file),
            files,
            cache: RefCell::new(HashMap::new()),
            loading: RefCell::new(HashMap::new()),
        }
    }

    pub fn metadata(&self) -> anyhow::Result<Metadata> {
        let (_, metadata) = read_mode_metadata(&mut *self.file.borrow_mut())?;

        Ok(metadata)
    }

    fn get_module_receiver(
        &self,
        module: &str,
    ) -> anyhow::Result<Option<tokio::sync::broadcast::Receiver<()>>> {
        if let Some(weak_sender) = self.loading.borrow().get(module) {
            if let Some(sender) = weak_sender.upgrade() {
                Ok(Some(sender.subscribe()))
            } else {
                bail!("Module {module} previously returned an error");
            }
        } else {
            Ok(None)
        }
    }

    pub async fn require(&self, lua: mlua::Lua, module: String) -> anyhow::Result<mlua::Value> {
        for path in decode_require(&module) {
            if let Some(source_file) = self.files.get(&path) {
                if let Some(mut receiver) = self.get_module_receiver(&path)? {
                    match receiver.recv().await {
                        Ok(()) => {}
                        Err(_) => bail!("Module {module} previously returned an error"),
                    }
                }

                if let Some(value) = self.cache.borrow().get(&path) {
                    return Ok(value.clone());
                }

                let (sender, _) = tokio::sync::broadcast::channel(1);

                self.loading
                    .borrow_mut()
                    .insert(path.clone(), sender.clone().downgrade());

                let file: String = read_source_file(&mut *self.file.borrow_mut(), source_file)?;

                let result: mlua::Value = lua
                    .load(file)
                    .set_name(format!("@{path}"))
                    .eval_async()
                    .await?;

                let final_value = if result.is_nil() {
                    mlua::Value::Boolean(true)
                } else {
                    result
                };

                self.cache.borrow_mut().insert(path, final_value.clone());

                let _ = sender.send(());

                return Ok(final_value);
            }
        }

        bail!("module '{module}' not found");
    }

    pub fn load(&self, lua: &mlua::Lua, path: String) -> anyhow::Result<mlua::Chunk<'static>> {
        if let Some(source_file) = self.files.get(&path) {
            let file: String = read_source_file(&mut *self.file.borrow_mut(), source_file)?;

            Ok(lua.load(file).set_name(format!("@{path}")))
        } else {
            bail!("File {path} not found");
        }
    }
}

fn decode_require(module: &str) -> Vec<String> {
    let path = module.replace(".", "/");

    return vec![format!("{path}.lua"), format!("{path}/init.lua")];
}
