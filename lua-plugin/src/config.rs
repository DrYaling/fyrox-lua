use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum BindingMode {
    #[default]
    EditorReflection,
    PackageFull,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LuaConfig {
    pub script_root: PathBuf,
    pub enabled: bool,
    pub binding_mode: BindingMode,
}
impl Default for LuaConfig {
    fn default() -> Self {
        Self {
            script_root: "data/scripts".into(),
            enabled: true,
            binding_mode: BindingMode::EditorReflection,
        }
    }
}
impl LuaConfig {
    pub fn effective_binding_mode(&self) -> BindingMode {
        if cfg!(feature = "editor") {
            self.binding_mode
        } else {
            BindingMode::PackageFull
        }
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let p = path.as_ref();
        if !p.exists() {
            return Ok(Self::default());
        }
        Ok(toml::from_str(&fs::read_to_string(p)?)?)
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
        let p = path.as_ref();
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?
        }
        fs::write(p, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
