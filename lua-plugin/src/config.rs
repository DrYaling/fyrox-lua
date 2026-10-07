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

/// Controls how a recovered Lua callback error affects later callbacks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LuaErrorPolicy {
    /// Stop dispatching the runtime and let the host destroy it on its normal
    /// lifecycle path.
    StopRuntime,
    /// Disable only the failing script instance (the production default).
    DisableScript,
    /// Keep dispatching and only disable an instance after the configured
    /// per-script error limit is reached.
    LogAndContinue,
}

impl Default for LuaErrorPolicy {
    fn default() -> Self {
        Self::DisableScript
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LuaConfig {
    pub script_root: PathBuf,
    pub enabled: bool,
    pub binding_mode: BindingMode,
    pub error_policy: LuaErrorPolicy,
    pub max_errors_per_script: u32,
    /// Maximum source size accepted by runtime-owned script loading paths.
    /// This bounds accidental or hostile allocations before parsing Lua.
    pub max_script_bytes: usize,
}
impl Default for LuaConfig {
    fn default() -> Self {
        Self {
            script_root: "data/scripts".into(),
            enabled: true,
            binding_mode: BindingMode::EditorReflection,
            error_policy: LuaErrorPolicy::default(),
            max_errors_per_script: 3,
            max_script_bytes: 4 * 1024 * 1024,
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
