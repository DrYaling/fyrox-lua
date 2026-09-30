//! Lua 脚本文件枚举工具。
//!
//! 这里仅负责按目录加载脚本文件，不分析 Lua 源码，也不参与绑定裁剪。
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn collect_lua_files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lua_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "lua") {
            output.push(path);
        }
    }
    output.sort();
}
