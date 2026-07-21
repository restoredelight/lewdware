use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
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
    /// Checks for circular requires
    loading: RefCell<HashSet<String>>,
}

impl Mode {
    pub fn new(file: Box<dyn ReadSeek>, files: HashMap<String, SourceFile>) -> Self {
        Self {
            file: RefCell::new(file),
            files,
            cache: RefCell::new(HashMap::new()),
            loading: RefCell::new(HashSet::new()),
        }
    }

    #[allow(unused)]
    pub fn metadata(&self) -> anyhow::Result<Metadata> {
        let (_, metadata) = read_mode_metadata(&mut *self.file.try_borrow_mut()?)?;

        Ok(metadata)
    }

    pub fn require(&self, lua: &mlua::Lua, module: String) -> anyhow::Result<mlua::Value> {
        for path in decode_require(&module) {
            if let Some(source_file) = self.files.get(&path) {
                if let Some(value) = self.cache.try_borrow()?.get(&path) {
                    return Ok(value.clone());
                }

                if !self.loading.try_borrow_mut()?.insert(path.clone()) {
                    bail!("circular require of module '{module}'");
                }

                let result = (|| -> anyhow::Result<mlua::Value> {
                    let file: String =
                        read_source_file(&mut *self.file.try_borrow_mut()?, source_file)?;

                    let result: mlua::Value = lua
                        .load(file)
                        .set_mode(mlua::ChunkMode::Text)
                        .set_name(format!("@{path}"))
                        .eval()?;

                    Ok(result)
                })();

                self.loading.try_borrow_mut()?.remove(&path);

                let final_value = match result? {
                    mlua::Value::Nil => mlua::Value::Boolean(true),
                    result => result,
                };

                self.cache
                    .try_borrow_mut()?
                    .insert(path, final_value.clone());

                return Ok(final_value);
            }
        }

        bail!("module '{module}' not found");
    }

    pub fn load(&self, lua: &mlua::Lua, path: String) -> anyhow::Result<mlua::Chunk<'static>> {
        if let Some(source_file) = self.files.get(&path) {
            let file: String = read_source_file(&mut *self.file.try_borrow_mut()?, source_file)?;

            Ok(lua
                .load(file)
                .set_mode(mlua::ChunkMode::Text)
                .set_name(format!("@{path}")))
        } else {
            bail!("File {path} not found");
        }
    }
}

fn decode_require(module: &str) -> Vec<String> {
    let path = module.replace(".", "/");

    vec![format!("{path}.lua"), format!("{path}/init.lua")]
}
