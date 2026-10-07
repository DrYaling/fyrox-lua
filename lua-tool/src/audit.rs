//! Generic serialized UI contract auditing.
//!
//! The auditor knows only the contract schema. Project or product names,
//! resource roots, and required node names are supplied by TOML.

use crate::config::UiContractConfig;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
struct UiContract {
    schema: u32,
    root: String,
    screens: Vec<UiScreenResult>,
}

#[derive(Debug, Serialize)]
struct UiScreenResult {
    name: String,
    path: String,
    expected_names: Vec<String>,
    found_names: Vec<String>,
    missing: Vec<String>,
    duplicate: Vec<String>,
    success: bool,
}

pub fn audit_ui_contract(
    config: UiContractConfig,
    config_base: &Path,
    output: &Path,
    atomic_write: impl Fn(&Path, &[u8]) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = resolve_path(config_base, &config.root);
    if !root.is_dir() {
        return Err(format!("UI contract root is not a directory: {}", root.display()).into());
    }
    let mut screens = Vec::with_capacity(config.screens.len());
    for screen in config.screens {
        let path = resolve_path(&root, &screen.file);
        let source =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let found_names = extract_ui_names(&source);
        let mut counts = BTreeMap::<String, usize>::new();
        for name in &found_names {
            *counts.entry(name.clone()).or_default() += 1;
        }
        let missing = screen
            .expected_names
            .iter()
            .filter(|name| !counts.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>();
        let duplicate = counts
            .iter()
            // 只校验合同声明的资源名；资源中其他示例节点的重复不属于
            // 当前业务合同，不能阻断无关页面的生成。
            .filter(|(name, count)| {
                **count > 1
                    && screen
                        .expected_names
                        .iter()
                        .any(|expected| expected == *name)
            })
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        screens.push(UiScreenResult {
            name: screen.name,
            path: path.to_string_lossy().replace('\\', "/"),
            expected_names: screen.expected_names,
            found_names,
            success: missing.is_empty() && duplicate.is_empty(),
            missing,
            duplicate,
        });
    }
    let contract = UiContract {
        schema: 1,
        root: root.to_string_lossy().replace('\\', "/"),
        screens,
    };
    atomic_write(
        &output.join(config.json_output),
        &serde_json::to_vec_pretty(&contract)?,
    )?;
    let mut markdown = String::from("# UI Contract Audit\n\n");
    for screen in &contract.screens {
        let missing = if screen.missing.is_empty() {
            "none".to_owned()
        } else {
            screen.missing.join(", ")
        };
        let duplicate = if screen.duplicate.is_empty() {
            "none".to_owned()
        } else {
            screen.duplicate.join(", ")
        };
        markdown.push_str(&format!(
            "## {}\n\n- path: `{}`\n- expected: {}\n- found: {}\n- missing: {}\n- duplicate: {}\n- result: **{}**\n\n",
            screen.name,
            screen.path,
            screen.expected_names.len(),
            screen.found_names.len(),
            missing,
            duplicate,
            if screen.success { "PASS" } else { "FAIL" },
        ));
    }
    atomic_write(&output.join(config.markdown_output), markdown.as_bytes())?;
    if contract.screens.iter().all(|screen| screen.success) {
        println!(
            "UI contract audit passed ({} screens)",
            contract.screens.len()
        );
        Ok(())
    } else {
        Err("UI contract audit failed; see generated contract output".into())
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

pub fn extract_ui_names(source: &str) -> Vec<String> {
    let marker = "Name<str:\"";
    let mut names = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find(marker) {
        rest = &rest[start + marker.len()..];
        let Some(end) = rest.find("\">") else { break };
        if !rest[..end].is_empty() {
            names.push(rest[..end].to_owned());
        }
        rest = &rest[end + 2..];
    }
    names
}
