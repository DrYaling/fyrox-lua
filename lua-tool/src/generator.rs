//! Configuration orchestration for catalog and business registration outputs.

use crate::config::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn generate_scan(input: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let (catalog, unsupported) =
        crate::generator_scan::scan_inputs(&[input.to_owned()], input.display().to_string())?;
    crate::generator_scan::write_catalog_artifacts(output, &catalog, &unsupported)?;
    Ok(())
}

pub fn generate_from_config(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let config: BindingConfig = toml::from_str(&fs::read_to_string(config_path)?)?;
    if config.schema != 1 {
        return Err(format!("unsupported lua binding config schema: {}", config.schema).into());
    }
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    for target in config.targets {
        if target.runtime_output.is_some() {
            return Err(format!(
                "target '{}' uses runtime_output; engine bindings are owned by lua-plugin and cannot be generated from the business config",
                target.name
            )
            .into());
        }
        if target.profile != "none" {
            return Err(format!(
                "target '{}' uses profile '{}'; config-driven targets must use profile = \"none\" because engine bindings are registered by lua-plugin",
                target.name, target.profile
            )
            .into());
        }
        let inputs = target
            .inputs
            .iter()
            .map(|path| crate::generator_io::resolve_config_path(base, path))
            .collect::<Vec<_>>();
        let output = crate::generator_io::resolve_config_path(base, &target.output);
        let input_display = inputs
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        generate_target(&target.name, &inputs, &output)?;
        match (target.api_output.as_ref(), target.api.as_ref()) {
            (Some(api_output), Some(api)) => crate::generator_api::write_api_registry(
                &crate::generator_io::resolve_config_path(base, api_output),
                &target.name,
                api,
            )?,
            (Some(_), None) => {
                return Err(format!(
                    "target '{}' has api_output but no [targets.api] configuration",
                    target.name
                )
                .into())
            }
            _ => {}
        }
        println!(
            "generated Lua target '{}' from {}",
            target.name,
            input_display.join(", ")
        );
    }
    if let Some(ui_contract) = config.ui_contract {
        let audit_output = ui_contract
            .output
            .as_ref()
            .map(|path| crate::generator_io::resolve_config_path(base, path))
            .unwrap_or_else(|| base.join("../target/lua-bindings/ui-audit"));
        fs::create_dir_all(&audit_output)?;
        crate::audit::audit_ui_contract(
            ui_contract,
            base,
            &audit_output,
            crate::generator_io::atomic_write,
        )?;
    }
    Ok(())
}

fn generate_target(
    name: &str,
    inputs: &[PathBuf],
    output: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let source = inputs
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let (catalog, unsupported) =
        crate::generator_scan::scan_inputs(inputs, format!("target '{name}' ({source})"))?;
    crate::generator_scan::write_catalog_artifacts(output, &catalog, &unsupported)
}
