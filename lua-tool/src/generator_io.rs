use std::{
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

/// Writes generated artifacts through a sibling temporary file before replacing
/// the destination. A failed generation leaves the last known-good file intact.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("out"),
        std::process::id()
    ));
    fs::write(&temp, bytes)?;
    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(_rename_error) if path.exists() => {
            fs::remove_file(path)?;
            fs::rename(&temp, path)?;
            Ok(())
        }
        Err(rename_error) => {
            let _ = fs::remove_file(&temp);
            Err(Box::new(rename_error))
        }
    }
}

pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub fn resolve_config_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

pub fn collect_rs(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path.to_owned());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let child = entry?.path();
        if child.is_dir() {
            collect_rs(&child, files)?;
        } else if child.extension().is_some_and(|ext| ext == "rs") {
            files.push(child);
        }
    }
    files.sort();
    files.dedup();
    Ok(())
}
