//! Creates an empty project-owned Lua tool manifest when one is missing.
//!
//! The plugin does not provide project binding or UI contract definitions.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

const DEFAULT_LUA_BINDINGS_CONFIG: &str = r#"# Project-owned Lua binding manifest.
# The editor creates this empty template once; add project entries as needed.
schema = 1
"#;

/// Initializes the project's editor Lua configuration without replacing an
/// existing binding manifest. Returns whether the default file was created.
pub fn initialize_editor_lua(project_root: impl AsRef<Path>) -> io::Result<bool> {
    let path = project_root
        .as_ref()
        .join("data/editor/lua/lua-bindings.toml");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(DEFAULT_LUA_BINDINGS_CONFIG.as_bytes())?;
            file.sync_all()?;
            fyrox::core::log::Log::info(format!(
                "[Lua] created default binding config at {}",
                path.display()
            ));
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{initialize_editor_lua, DEFAULT_LUA_BINDINGS_CONFIG};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_manifest_is_generic_and_has_no_project_symbols() {
        assert!(DEFAULT_LUA_BINDINGS_CONFIG.contains("schema = 1"));
        let manifest: toml::Value = toml::from_str(DEFAULT_LUA_BINDINGS_CONFIG).unwrap();
        assert!(manifest.get("targets").is_none());
        assert!(manifest.get("ui_contract").is_none());
    }

    #[test]
    fn project_manifest_is_created_once_and_never_overwritten() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("fwok-lua-config-test-{suffix}"));
        std::fs::create_dir_all(&root).expect("temporary project root");
        assert!(initialize_editor_lua(&root).expect("create config"));
        let path = root.join("data/editor/lua/lua-bindings.toml");
        let parsed: toml::Value = toml::from_str(&DEFAULT_LUA_BINDINGS_CONFIG).expect("valid TOML");
        assert_eq!(parsed["schema"].as_integer(), Some(1));
        assert!(parsed.get("targets").is_none());
        assert_eq!(
            std::fs::read_to_string(&path).expect("read config"),
            DEFAULT_LUA_BINDINGS_CONFIG
        );
        std::fs::write(&path, "custom = true\n").expect("customize config");
        assert!(!initialize_editor_lua(&root).expect("preserve config"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read preserved config"),
            "custom = true\n"
        );
        std::fs::remove_dir_all(root).expect("cleanup temporary project root");
    }
}
