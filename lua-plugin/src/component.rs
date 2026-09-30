//! LuaComponent 的引擎组件引用。
//!
//! 配置数据必须可被 Fyrox Inspector/Visit 序列化，因此使用 Vec 保存；运行时
//! 由 `rebuild_index` 构建 HashMap，避免每次按 key 线性扫描。

use fyrox::core::{pool::ErasedHandle, reflect::prelude::*, visitor::prelude::*};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Reflect, Visit)]
#[reflect(type_uuid = "c0d36c5c-ae6c-4c54-9a0c-2e8b6e7e1001")]
pub enum ComponentType {
    Node,
    Node2D,
    Node3D,
    Button,
    Text,
    TextBox,
    Image,
    Custom(String),
}

impl Default for ComponentType {
    fn default() -> Self {
        Self::Custom(String::new())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Reflect, Visit)]
#[reflect(type_uuid = "c0d36c5c-ae6c-4c54-9a0c-2e8b6e7e1002")]
pub struct ComponentBinding {
    pub component_type: ComponentType,
    pub handle: ErasedHandle,
    pub key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Reflect, Visit)]
#[reflect(type_uuid = "c0d36c5c-ae6c-4c54-9a0c-2e8b6e7e1003")]
pub struct ComponentBindings {
    pub entries: Vec<ComponentBinding>,
    #[visit(skip)]
    #[reflect(hidden)]
    index: HashMap<String, usize>,
}

impl ComponentBindings {
    pub fn rebuild_index(&mut self) {
        self.index.clear();
        for (position, entry) in self.entries.iter().enumerate() {
            if !entry.key.is_empty() {
                self.index.insert(entry.key.clone(), position);
            }
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&ComponentBinding> {
        if self.index.len() != self.entries.len() {
            self.rebuild_index();
        }
        self.index.get(key).and_then(|i| self.entries.get(*i))
    }
}
