use std::{fs, path::Path};

use anyhow::Result;

pub const API_STUBS: &str = include_str!("../../../shared/src/lua/api.lua");

pub fn write_type_stubs(root: &Path) -> Result<()> {
    let types_dir = root.join(".types");
    fs::create_dir_all(&types_dir)?;
    fs::write(types_dir.join("lewdware.d.lua"), API_STUBS)?;
    Ok(())
}

pub fn types() -> Result<()> {
    let root = crate::mode::find_root()?;
    write_type_stubs(&root)?;
    println!("Updated .types/lewdware.d.lua");
    Ok(())
}
