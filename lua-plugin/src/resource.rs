use crate::component::ComponentBindings;
use fyrox::{
    asset::{
        io::ResourceIo,
        loader::{BoxedLoaderFuture, LoaderPayload, ResourceLoader},
        manager::ResourceManager,
        state::LoadError,
        Resource, ResourceData,
    },
    core::{reflect::prelude::*, uuid::Uuid, visitor::prelude::*, SafeLock},
    script::{ScriptContext, ScriptTrait},
};
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A Lua source file managed by Fyrox's resource system.
#[derive(Clone, Debug, PartialEq, Reflect, Visit)]
#[reflect(type_uuid = "e9a30b68-91d2-4cf7-a1bc-8af267f3c8a1")]
pub struct LuaScript {
    /// UTF-8 source code of the script.
    pub source: String,
}

impl Default for LuaScript {
    fn default() -> Self {
        Self {
            source: r#"local Component = {}
Component.__index = Component

function Component.new(class)
    return setmetatable({}, class)
end

function Component:on_awake()
end

function Component:start()
end

function Component:update(dt)
end

function Component:on_destroy()
end

return Component
"#
            .to_owned(),
        }
    }
}

impl ResourceData for LuaScript {
    fn save(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        std::fs::write(path, self.source.as_bytes())?;
        Ok(())
    }

    fn can_be_saved(&self) -> bool {
        true
    }

    fn try_clone_box(&self) -> Option<Box<dyn ResourceData>> {
        Some(Box::new(self.clone()))
    }
}

#[derive(Default, Copy, Clone, Debug)]
pub struct LuaScriptLoader;

impl ResourceLoader for LuaScriptLoader {
    fn extensions(&self) -> &[&str] {
        &["lua"]
    }

    fn data_type_uuid(&self) -> Uuid {
        <LuaScript as Reflect>::type_info().type_uuid
    }

    fn load(&self, path: PathBuf, io: Arc<dyn ResourceIo>) -> BoxedLoaderFuture {
        Box::pin(async move {
            let bytes = io.load_file(&path).await.map_err(LoadError::new)?;
            let source = String::from_utf8(bytes).map_err(|e| {
                LoadError::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            })?;
            Ok(LoaderPayload::new(LuaScript { source }))
        })
    }
}

/// Registers the Lua source resource loader and constructor with an engine.
///
/// This should be called by the host executable before loading a dynamic game
/// plugin. Keeping the loader in the host prevents a hot-reloaded plugin from
/// leaving a resource-manager vtable pointing into an unloaded DLL.
pub fn register_lua_resource_loader(resource_manager: &ResourceManager) {
    let state = resource_manager.state();
    let type_uuid = <LuaScript as Reflect>::type_info().type_uuid;
    if !state
        .constructors_container
        .map
        .safe_lock()
        .contains_key(&type_uuid)
    {
        state.constructors_container.add::<LuaScript>();
    }
    let mut loaders = state.loaders.safe_lock();
    if !loaders
        .iter()
        .any(|loader| loader.data_type_uuid() == type_uuid)
    {
        loaders.set(LuaScriptLoader);
    }
}

/// Registers Lua resources and the lightweight node component with an engine.
pub fn register_fyrox_resources(
    resource_manager: &ResourceManager,
    scripts: &fyrox::script::constructor::ScriptConstructorContainer,
) {
    register_lua_resource_loader(resource_manager);
    let type_uuid = <LuaComponent as Reflect>::type_info().type_uuid;
    if !scripts.map().contains_key(&type_uuid) {
        scripts.add::<LuaComponent>("Lua Component");
    }
}

/// Inspector 中可编辑的 Lua 参数。
///
/// 参数采用稳定的字符串表示，避免在资源序列化层引入 Lua 值或 Rust 借用；
/// 脚本侧可以按需使用 `tonumber`、布尔转换等方式解析。
#[derive(Clone, Debug, PartialEq, Reflect, Visit)]
#[reflect(type_uuid = "b9a30d1f-7d71-46be-9f77-3c0a8ef3d6e5")]
pub struct LuaScriptParameter {
    /// Lua 脚本中使用的参数名。
    pub name: String,
    /// Inspector 保存的文本值。
    pub value: String,
    /// 是否将该参数导出到 Lua 实例的 `params` 表。
    pub exported: bool,
}

impl Default for LuaScriptParameter {
    fn default() -> Self {
        Self {
            name: "parameter".to_owned(),
            value: String::new(),
            exported: true,
        }
    }
}

/// LuaComponent 的 EditorScript 配置。
///
/// 该结构专门承载 Inspector 参数，不保存运行时 Lua 状态。编辑器修改后，
/// 宿主在创建脚本实例时可将 `parameters` 转换为 Lua table 传入 `new(class, params)`。
#[derive(Clone, Debug, PartialEq, Reflect, Visit)]
#[reflect(type_uuid = "f2f25e4f-d3c4-4d8b-b0ec-3e3cbbca5916")]
pub struct EditorScript {
    /// 是否在编辑器预览和运行时启用脚本。
    pub enabled: bool,
    /// Inspector 中显示的脚本参数列表。
    pub parameters: Vec<LuaScriptParameter>,
}

impl Default for EditorScript {
    fn default() -> Self {
        Self {
            enabled: true,
            parameters: Vec::new(),
        }
    }
}

impl EditorScript {
    /// 返回导出参数，供 LuaComponent 创建实例时使用。
    pub fn exported_parameters(&self) -> impl Iterator<Item = (&str, &str)> {
        self.parameters
            .iter()
            .filter(|parameter| parameter.exported && !parameter.name.is_empty())
            .map(|parameter| (parameter.name.as_str(), parameter.value.as_str()))
    }
}

/// A small serializable script component. The selected Lua source is a regular resource.
#[derive(Clone, Debug, PartialEq, Reflect, Visit)]
#[reflect(type_uuid = "ed5aa1a8-9a7d-47b7-a3cb-a2f8f8fbcc0b")]
pub struct LuaComponent {
    /// Lua source selected for this node.
    pub script: Option<Resource<LuaScript>>,
    /// Inspector 中可直接编辑的脚本内容。为空时使用 `script` 资源内容。
    /// 该字段用于快速绑定和原型调试，不会修改外部资源文件。
    pub source_override: String,
    /// Whether the component should execute when a runtime bridge is attached.
    pub enabled: bool,
    /// EditorScript 参数；该字段会直接显示在 Fyrox Inspector 中。
    pub editor_script: EditorScript,
    /// 绑定到脚本实例的引擎组件。配置保存为 Vec，运行时建立快速索引。
    pub components: ComponentBindings,
}

impl Default for LuaComponent {
    fn default() -> Self {
        Self {
            script: None,
            source_override: String::new(),
            enabled: true,
            editor_script: EditorScript::default(),
            components: ComponentBindings::default(),
        }
    }
}

impl ScriptTrait for LuaComponent {
    fn on_init(&mut self, _ctx: &mut ScriptContext) -> fyrox::plugin::error::GameResult {
        Ok(())
    }
}
