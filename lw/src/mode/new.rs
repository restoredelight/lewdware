use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};

use anyhow::Result;
use include_dir::{Dir, DirEntry, include_dir};

use super::types::write_type_stubs;

static DEFAULT_SRC: Dir = include_dir!("$CARGO_MANIFEST_DIR/../default-modes/src");
const DEFAULT_CONFIG_JSONC: &str = include_str!("../../../default-modes/config.jsonc");

fn copy_dir(dir: &Dir, dest: &PathBuf) -> Result<()> {
    for entry in dir.entries() {
        match entry {
            DirEntry::File(file) => {
                let path = dest.join(file.path());
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, file.contents())?;
            }
            DirEntry::Dir(subdir) => copy_dir(subdir, dest)?,
        }
    }
    Ok(())
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(d) if !d.is_empty() => print!("{} [{}]: ", label, d),
        _ => print!("{}: ", label),
    }
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed.is_empty() {
        Ok(default.unwrap_or("").to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn create_new_mode(from_default: bool) -> Result<()> {
    let name = prompt("Name", None)?;
    if name.is_empty() {
        anyhow::bail!("Name is required");
    }
    let author = prompt("Author", None)?;
    let version = prompt("Version", Some("0.1.0"))?;

    let base_path = PathBuf::from(&name);
    if base_path.exists() {
        anyhow::bail!("Directory '{}' already exists", name);
    }

    fs::create_dir_all(base_path.join("src"))?;

    if from_default {
        let escaped_name = json_escape(&name);
        let escaped_author = json_escape(&author);
        let escaped_version = json_escape(&version);

        let config_content = DEFAULT_CONFIG_JSONC
            .replace(
                "\"$schema\": \"../docs/src/data/config.schema.json\",\n  // \"$schema\": \"https://lewdware.github.com/config.schema.json\",",
                "\"$schema\": \"https://lewdware.net/reference/config.schema.json\",",
            )
            .replace("\"Default Modes\"", &format!("\"{}\"", escaped_name))
            .replace("\"restoredelight\"", &format!("\"{}\"", escaped_author))
            .replace("\"0.1.0\"", &format!("\"{}\"", escaped_version));

        fs::write(base_path.join("config.jsonc"), config_content)?;
        copy_dir(&DEFAULT_SRC, &base_path.join("src"))?;
    } else {
        let escaped_name = json_escape(&name);
        let escaped_author = json_escape(&author);
        let escaped_version = json_escape(&version);

        let config_content = format!(
            r#"{{
  "$schema": "https://lewdware.net/reference/config.schema.json",
  "name": "{escaped_name}",
  "version": "{escaped_version}",
  "author": "{escaped_author}",
  "include": ["src"],
  "modes": {{
    "default": {{
      "name": "{escaped_name}",
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
    }

    write_type_stubs(&base_path)?;

    let luarc_content = r#"{
  "$schema": "https://raw.githubusercontent.com/LuaLS/vscode-lua/master/setting/schema.json",
  "runtime.version": "Lua 5.5",
  "workspace.library": ["src", ".types/lewdware.d.lua"],
  "diagnostics.globals": ["lewdware"]
}"#;
    fs::write(base_path.join(".luarc.json"), luarc_content)?;

    fs::write(base_path.join(".gitignore"), "build\n.types\n")?;

    println!("Mode '{}' created successfully!", name);

    Ok(())
}
