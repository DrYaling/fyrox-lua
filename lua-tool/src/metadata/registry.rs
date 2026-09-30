//! 手写绑定注册表及其审计元数据。
use std::collections::BTreeMap;

#[derive(serde::Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingCategory {
    Core,
    Engine,
    Scene,
    Ui,
    Input,
    Resource,
    Physics,
    Rendering,
    Material,
    Animation,
    Audio,
    Scripting,
    Scene2D,
    Scene3D,
}

/// 绑定条目的实现状态：目录登记不会向 Lua 暴露不存在的函数。
#[derive(serde::Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingStatus {
    /// 已注册 Rust wrapper，可由 Lua 直接调用。
    Implemented,
    /// 已登记公开类型和接口，等待手写 wrapper 实现。
    CatalogOnly,
}

#[derive(serde::Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BindingMethod {
    pub name: &'static str,
    pub signature: &'static str,
    pub description: &'static str,
}

#[derive(serde::Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BindingType {
    pub name: &'static str,
    pub module: &'static str,
    pub category: BindingCategory,
    pub description: &'static str,
    pub status: BindingStatus,
    pub methods: Vec<BindingMethod>,
}

#[derive(serde::Serialize, Debug, Clone, Default)]
pub struct BindingRegistry {
    types: BTreeMap<&'static str, BindingType>,
}

impl BindingRegistry {
    pub fn register_type(&mut self, binding: BindingType) -> Result<(), String> {
        if self.types.insert(binding.name, binding).is_some() {
            return Err("重复注册 Lua 引擎类型".to_owned());
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&BindingType> {
        self.types.get(name)
    }

    pub fn types(&self) -> impl Iterator<Item = &BindingType> {
        self.types.values()
    }

    /// 返回指定领域的公开类型，供编辑器目录和绑定审计使用。
    pub fn by_category(&self, category: BindingCategory) -> impl Iterator<Item = &BindingType> {
        self.types
            .values()
            .filter(move |item| item.category == category)
    }

    /// 返回已提供 Rust wrapper 的类型数量。
    pub fn implemented_len(&self) -> usize {
        self.types
            .values()
            .filter(|item| item.status == BindingStatus::Implemented)
            .count()
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}
