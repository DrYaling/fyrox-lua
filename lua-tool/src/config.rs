use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    pub schema: u32,
    #[serde(default)]
    pub targets: Vec<BindingTargetConfig>,
    #[serde(default)]
    pub ui_contract: Option<UiContractConfig>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingTargetConfig {
    pub name: String,
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    #[serde(default)]
    pub runtime_output: Option<PathBuf>,
    #[serde(default)]
    pub api_output: Option<PathBuf>,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default)]
    pub api: Option<ApiConfig>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    #[serde(default)]
    pub type_name: Option<String>,
    #[serde(default)]
    pub context: Vec<ApiContextField>,
    #[serde(default)]
    pub registrations: Vec<ApiRegistration>,
    #[serde(default)]
    pub custom_bindings: Vec<ApiRegistration>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiContextField {
    pub name: String,
    #[serde(rename = "type")]
    pub rust_type: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiRegistration {
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiContractConfig {
    pub root: PathBuf,
    #[serde(default = "default_json_name")]
    pub json_output: String,
    #[serde(default = "default_markdown_name")]
    pub markdown_output: String,
    #[serde(default)]
    pub output: Option<PathBuf>,
    pub screens: Vec<UiScreenConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiScreenConfig {
    pub name: String,
    pub file: PathBuf,
    #[serde(default)]
    pub expected_names: Vec<String>,
}

fn default_json_name() -> String {
    "ui-contract.json".to_owned()
}
fn default_markdown_name() -> String {
    "ui-contract.md".to_owned()
}
pub fn default_profile() -> String {
    "none".to_owned()
}
