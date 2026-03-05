use std::{fs, path::PathBuf};

use anyhow::Result;

const API_STUBS: &str = include_str!("../../../shared/src/lua/api.lua");

pub fn create_new_mode(name: &str) -> Result<()> {
    let base_path = PathBuf::from(name);
    if base_path.exists() {
        anyhow::bail!("Directory '{}' already exists", name);
    }

    println!("Creating new mode: {}", name);

    fs::create_dir_all(base_path.join("src"))?;

    let config_content = format!(
        r#"{{
  "$schema": "https://lewdware.github.com/mode.schema.json",
  "name": "{name}",
  "version": "0.1.0",
  "author": "",
  "include": ["src"],
  "modes": {{
    "default": {{
      "name": "{name}",
      "entrypoint": "src/main.lua",
      "options": {{}}
    }}
  }}
}}"#
    );
    fs::write(base_path.join("config.jsonc"), config_content)?;

    let lua_content = r#"lewdware.every(1000, function()
    local media = lewdware.media.random({ type = {"image", "video"} });
    if media then
        if media.type == "image" then
            lewdware.spawn_image_popup(media)
        elseif media.type == "video" then
            lewdware.spawn_video_popup(media)
        end
    end
end)
"#;
    fs::write(base_path.join("src/main.lua"), lua_content)?;

    fs::create_dir(base_path.join(".types"))?;
    fs::write(base_path.join(".types/lewdware.lua"), API_STUBS)?;

    let luarc_content = r#"{
  "$schema": "https://raw.githubusercontent.com/LuaLS/vscode-lua/master/setting/schema.json",
  "runtime.version": "Lua 5.1",
  "workspace.library": ["src", ".types/lewdware.lua"],
  "diagnostics.globals": ["lewdware"]
}"#;
    fs::write(base_path.join(".luarc.json"), luarc_content)?;

    let gitingore_content = r#"build
"#;
    fs::write(base_path.join(".gitignore"), gitingore_content)?;

    println!("Mode '{}' created successfully!", name);

    Ok(())
}
