use crate::handles::HandleToken;
use crate::{
    bindings::{register_engine_bindings, register_generated_component_aliases},
    config::{BindingMode, LuaConfig},
    resource::{EditorScript, LuaComponent},
    script_files::collect_lua_files,
    LuaGameApi,
};
use mlua::{Function, Lua, RegistryKey, Table, Value};
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    thread::ThreadId,
};

macro_rules! lua_info {
    ($($arg:tt)*) => {
        fyrox::core::log::Log::info(format!($($arg)*));
    };
}
macro_rules! lua_warn {
    ($($arg:tt)*) => {
        fyrox::core::log::Log::warn(format!($($arg)*));
    };
}

pub struct LuaRuntime {
    pub lua: Lua,
    scripts: Vec<ScriptInstance>,
    update_indices: Vec<usize>,
    main: Option<MainScript>,
    config: LuaConfig,
    state: RuntimeState,
    next_script_id: u64,
    scope_tables: RefCell<HashMap<HandleToken, RegistryKey>>,
    update_ticks: u64,
    events_dispatched: u64,
    owner_thread: ThreadId,
    // Rc is intentionally !Send + !Sync: a LuaRuntime belongs to one Fyrox thread.
    _main_thread_only: Rc<()>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeState {
    Created,
    Started,
    Running,
    Stopping,
    Stopped,
}
struct ScriptInstance {
    path: PathBuf,
    instance: RegistryKey,
    id: u64,
    awake: Option<RegistryKey>,
    start: Option<RegistryKey>,
    update: Option<RegistryKey>,
    event: Option<RegistryKey>,
    destroy: Option<RegistryKey>,
    scope: Option<HandleToken>,
}
struct MainScript {
    instance: RegistryKey,
    scripts: RegistryKey,
    on_awake: Option<RegistryKey>,
    start: Option<RegistryKey>,
    update: Option<RegistryKey>,
    on_destroy: Option<RegistryKey>,
}
impl std::fmt::Debug for LuaRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaRuntime")
            .field("scripts", &self.scripts.len())
            .finish()
    }
}
impl PartialEq for LuaRuntime {
    fn eq(&self, o: &Self) -> bool {
        std::ptr::eq(self, o)
    }
}
impl LuaRuntime {
    pub fn new(config: LuaConfig, api: impl LuaGameApi) -> mlua::Result<Self> {
        Self::new_inner(config, api, true)
    }

    /// Creates a runtime that combines project scripts (including `main.lua`) with
    /// scene-owned LuaComponent instances in one VM. Root class scripts are loaded
    /// by `LuaPluginHost::start_scene`; direct callers can opt into them with `new`.
    pub fn new_for_scene(config: LuaConfig, api: impl LuaGameApi) -> mlua::Result<Self> {
        Self::new_inner(config, api, false)
    }

    fn new_inner(config: LuaConfig, api: impl LuaGameApi, load_root: bool) -> mlua::Result<Self> {
        let root = config.script_root.clone();
        lua_info!(
            "[Lua] runtime initializing (root={}, binding={:?})",
            root.display(),
            config.effective_binding_mode()
        );
        let lua = Lua::new();
        // 让场景脚本可以通过标准 require 互相组合；根目录由配置决定。
        if let Ok(package) = lua.globals().get::<Table>("package") {
            let path = format!("{}/?.lua;{}/?/init.lua", root.display(), root.display());
            package.set("path", path)?;
        }
        // 引擎绑定只从手写注册入口装载；这里绝不扫描 Lua 源码。
        register_engine_bindings(&lua)?;
        lua_info!(
            "[Lua] API registration complete (mode={:?}, executable bindings)",
            config.binding_mode
        );
        api.register(&lua)?;
        register_generated_component_aliases(&lua)?;
        let mut runtime = Self {
            lua,
            scripts: vec![],
            update_indices: vec![],
            main: None,
            config,
            state: RuntimeState::Created,
            next_script_id: 1,
            scope_tables: RefCell::new(HashMap::new()),
            update_ticks: 0,
            events_dispatched: 0,
            owner_thread: std::thread::current().id(),
            _main_thread_only: Rc::new(()),
        };
        runtime.load_main(&root)?;
        if load_root {
            runtime.load_root(&root)?;
        }
        lua_info!(
            "[Lua] runtime initialized with {} script(s)",
            runtime.scripts.len()
        );
        Ok(runtime)
    }
    pub fn binding_mode(&self) -> BindingMode {
        self.assert_owner_thread();
        self.config.effective_binding_mode()
    }
    pub fn config(&self) -> &LuaConfig {
        self.assert_owner_thread();
        &self.config
    }

    #[inline]
    fn assert_owner_thread(&self) {
        assert_eq!(
            self.owner_thread,
            std::thread::current().id(),
            "LuaRuntime used from a non-owner thread"
        );
    }
    pub fn reload_script(&mut self, path: &Path) -> mlua::Result<()> {
        self.assert_owner_thread();
        if let Some(i) = self.scripts.iter().position(|entry| entry.path == path) {
            let old = self.scripts.remove(i);
            let destroy_result = self.call_no_arg(&old, old.destroy.as_ref());
            self.remove_script_instance(&old.path)?;
            self.remove_instance_keys(old)?;
            self.rebuild_update_indices();
            destroy_result?;
        }
        self.load_script(path)
    }
    pub fn load_root(&mut self, root: &Path) -> mlua::Result<()> {
        self.assert_owner_thread();
        let mut files = vec![];
        collect_lua_files(root, &mut files);
        lua_info!(
            "[Lua] script root '{}': discovered {} file(s)",
            root.display(),
            files.len()
        );
        if files.is_empty() {
            lua_warn!("Configured Lua script root is empty: {}", root.display());
        }
        for p in files {
            // 目录中的可复用模块由 Lua `require` 按需加载，不作为场景组件实例化。
            if p.components().any(|c| c.as_os_str() == "modules") || p == root.join("main.lua") {
                continue;
            }
            self.load_script(&p)?;
            lua_info!("[Lua] script loaded: {}", p.display());
        }
        Ok(())
    }

    /// Loads the optional project entry point. `main.lua` returns a table whose
    /// dot-style lifecycle functions are called in the same VM as component scripts.
    fn load_main(&mut self, root: &Path) -> mlua::Result<()> {
        let path = root.join("main.lua");
        if !path.is_file() {
            return Ok(());
        }
        let source = fs::read_to_string(&path).map_err(mlua::Error::external)?;
        let value: Value = self
            .lua
            .load(&source)
            .set_name(path.to_string_lossy())
            .eval()?;
        let class = match value {
            Value::Table(table) => table,
            _ => match self.lua.globals().get::<Table>("main") {
                Ok(table) => table,
                Err(_) => {
                    lua_warn!("main.lua did not return or assign a main table");
                    return Ok(());
                }
            },
        };
        let instance: Table = if let Ok(ctor) = class.get::<Function>("new") {
            ctor.call(class.clone())?
        } else {
            class
        };
        let scripts = self.lua.create_table()?;
        instance.set("scripts", scripts.clone())?;
        let entry = MainScript {
            on_awake: Self::method_key(&self.lua, &instance, "on_awake")?,
            start: Self::method_key(&self.lua, &instance, "start")?,
            update: Self::method_key(&self.lua, &instance, "update")?,
            on_destroy: Self::method_key(&self.lua, &instance, "on_destroy")?,
            scripts: self.lua.create_registry_value(scripts)?,
            instance: self.lua.create_registry_value(instance)?,
        };
        self.call_main_no_arg(entry.on_awake.as_ref(), &entry)
            .map_err(|error| {
                mlua::Error::runtime(format!("main.lua phase=on_awake error={error}"))
            })?;
        self.main = Some(entry);
        lua_info!("[Lua] main.lua loaded: {}", path.display());
        Ok(())
    }
    pub fn load_script(&mut self, p: &Path) -> mlua::Result<()> {
        self.assert_owner_thread();
        let source = fs::read_to_string(p).map_err(mlua::Error::external)?;
        self.load_script_source(p, &source, &EditorScript::default())
    }

    /// 从场景 LuaComponent 加载脚本。资源脚本和目录脚本共用同一生命周期实现。
    pub fn load_script_source(
        &mut self,
        id: &Path,
        source: &str,
        editor_script: &EditorScript,
    ) -> mlua::Result<()> {
        self.assert_owner_thread();
        self.load_script_source_with_components(
            id,
            source,
            editor_script,
            &Default::default(),
            None,
        )
    }

    fn load_script_source_with_components(
        &mut self,
        id: &Path,
        source: &str,
        editor_script: &EditorScript,
        component_bindings: &crate::component::ComponentBindings,
        scope: Option<HandleToken>,
    ) -> mlua::Result<()> {
        if !editor_script.enabled {
            return Ok(());
        }
        let class: Table = self
            .lua
            .load(source)
            .set_name(id.to_string_lossy())
            .eval()?;
        let ctor: Function = class
            .get("new")
            .map_err(|_| mlua::Error::runtime(format!("脚本 {} 必须定义 new()", id.display())))?;
        let params = self.lua.create_table()?;
        let components = self.lua.create_table()?;
        for binding in &component_bindings.entries {
            let value = self.lua.create_table()?;
            value.set("type", format!("{:?}", binding.component_type))?;
            value.set("index", binding.handle.index())?;
            value.set("generation", binding.handle.generation())?;
            components.set(binding.key.as_str(), value)?;
        }
        params.set("components", components)?;
        for (name, value) in editor_script.exported_parameters() {
            params.set(name, value)?;
        }
        let instance: Table = self.with_scope(scope, || ctor.call((class.clone(), params)))?;
        let entry = ScriptInstance {
            path: id.to_path_buf(),
            awake: Self::method_key(&self.lua, &instance, "on_awake")?,
            start: Self::method_key(&self.lua, &instance, "start")?,
            update: Self::method_key(&self.lua, &instance, "update")?,
            event: Self::method_key(&self.lua, &instance, "on_event")?,
            destroy: Self::method_key(&self.lua, &instance, "on_destroy")?,
            instance: self.lua.create_registry_value(instance)?,
            scope,
            id: self.next_script_id,
        };
        self.next_script_id = self.next_script_id.saturating_add(1);
        let has_update = entry.update.is_some();
        self.register_script_instance(id, &self.lua.registry_value(&entry.instance)?)?;
        if let Err(error) = self.call_no_arg(&entry, entry.awake.as_ref()) {
            self.remove_script_instance(id)?;
            self.remove_instance_keys(entry)?;
            return Err(mlua::Error::runtime(format!(
                "script path={} phase=on_awake error={error}",
                id.display()
            )));
        }
        lua_info!("[Lua] script instance ready: {}", id.display());
        self.scripts.push(entry);
        if has_update {
            self.update_indices.push(self.scripts.len() - 1);
        }
        Ok(())
    }

    fn method_key(lua: &Lua, instance: &Table, name: &str) -> mlua::Result<Option<RegistryKey>> {
        let method = instance.get::<Function>(name).ok();
        method.map(|f| lua.create_registry_value(f)).transpose()
    }

    fn function(
        &self,
        entry: &ScriptInstance,
        key: &RegistryKey,
    ) -> mlua::Result<(Table, Function)> {
        Ok((
            self.lua.registry_value(&entry.instance)?,
            self.lua.registry_value(key)?,
        ))
    }

    fn call_no_arg(&self, entry: &ScriptInstance, key: Option<&RegistryKey>) -> mlua::Result<()> {
        let Some(key) = key else { return Ok(()) };
        let (instance, function) = self.function(entry, key)?;
        self.with_scope(entry.scope, || function.call::<()>((instance,)))
            .map_err(|error| {
                mlua::Error::runtime(format!(
                    "script id={} path={} phase=callback error={error}",
                    entry.id,
                    entry.path.display()
                ))
            })
    }

    fn call_dt(
        &self,
        entry: &ScriptInstance,
        key: Option<&RegistryKey>,
        dt: f32,
    ) -> mlua::Result<()> {
        let Some(key) = key else { return Ok(()) };
        let (instance, function) = self.function(entry, key)?;
        self.with_scope(entry.scope, || function.call::<()>((instance, dt)))
            .map_err(|error| {
                mlua::Error::runtime(format!(
                    "script id={} path={} phase=update error={error}",
                    entry.id,
                    entry.path.display()
                ))
            })
    }

    fn call_main_no_arg(&self, key: Option<&RegistryKey>, entry: &MainScript) -> mlua::Result<()> {
        let Some(key) = key else { return Ok(()) };
        let instance: Table = self.lua.registry_value(&entry.instance)?;
        let function: Function = self.lua.registry_value(key)?;
        function
            .call::<()>(instance)
            .map_err(|error| mlua::Error::runtime(format!("main.lua phase=callback error={error}")))
    }

    fn call_main_dt(
        &self,
        key: Option<&RegistryKey>,
        entry: &MainScript,
        dt: f32,
    ) -> mlua::Result<()> {
        let Some(key) = key else { return Ok(()) };
        let instance: Table = self.lua.registry_value(&entry.instance)?;
        let function: Function = self.lua.registry_value(key)?;
        function
            .call::<()>((instance, dt))
            .map_err(|error| mlua::Error::runtime(format!("main.lua phase=update error={error}")))
    }

    fn remove_main_keys(&self, entry: MainScript) -> mlua::Result<()> {
        self.lua.remove_registry_value(entry.instance)?;
        self.lua.remove_registry_value(entry.scripts)?;
        for key in [entry.on_awake, entry.start, entry.update, entry.on_destroy]
            .into_iter()
            .flatten()
        {
            self.lua.remove_registry_value(key)?;
        }
        Ok(())
    }

    #[inline]
    fn script_key(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn register_script_instance(&self, path: &Path, instance: &Table) -> mlua::Result<()> {
        if let Some(main) = self.main.as_ref() {
            let scripts: Table = self.lua.registry_value(&main.scripts)?;
            scripts.set(Self::script_key(path), instance.clone())?;
        }
        Ok(())
    }

    fn remove_script_instance(&self, path: &Path) -> mlua::Result<()> {
        if let Some(main) = self.main.as_ref() {
            let scripts: Table = self.lua.registry_value(&main.scripts)?;
            scripts.set(Self::script_key(path), Value::Nil)?;
        }
        Ok(())
    }

    fn call_event(
        &self,
        entry: &ScriptInstance,
        key: Option<&RegistryKey>,
        name: &str,
        payload: &str,
    ) -> mlua::Result<()> {
        let Some(key) = key else { return Ok(()) };
        let (instance, function) = self.function(entry, key)?;
        self.with_scope(entry.scope, || {
            function.call::<()>((instance, name, payload))
        })
        .map_err(|error| {
            mlua::Error::runtime(format!(
                "script id={} path={} phase=event:{} error={error}",
                entry.id,
                entry.path.display(),
                name
            ))
        })
    }

    #[inline]
    fn with_scope<T>(
        &self,
        scope: Option<HandleToken>,
        callback: impl FnOnce() -> mlua::Result<T>,
    ) -> mlua::Result<T> {
        let globals = self.lua.globals();
        let previous: mlua::Value = globals.get("__fwok_current_scene_scope")?;
        match scope {
            Some(token) => {
                if !self.scope_tables.borrow().contains_key(&token) {
                    let table = self.lua.create_table()?;
                    table.set("index", token.index)?;
                    table.set("generation", token.generation)?;
                    let key = self.lua.create_registry_value(table)?;
                    self.scope_tables.borrow_mut().insert(token, key);
                }
                let table: Table = self
                    .lua
                    .registry_value(&self.scope_tables.borrow()[&token])?;
                table.set("index", token.index)?;
                table.set("generation", token.generation)?;
                globals.set("__fwok_current_scene_scope", table)?;
            }
            None => globals.set("__fwok_current_scene_scope", mlua::Value::Nil)?,
        }
        let result = callback();
        globals.set("__fwok_current_scene_scope", previous)?;
        result
    }

    fn remove_instance_keys(&self, entry: ScriptInstance) -> mlua::Result<()> {
        self.lua.remove_registry_value(entry.instance)?;
        for key in [
            entry.awake,
            entry.start,
            entry.update,
            entry.event,
            entry.destroy,
        ]
        .into_iter()
        .flatten()
        {
            self.lua.remove_registry_value(key)?;
        }
        Ok(())
    }

    fn rebuild_update_indices(&mut self) {
        self.update_indices = self
            .scripts
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.update.as_ref().map(|_| index))
            .collect();
    }

    /// 加载场景中的 LuaComponent。组件只保存资源引用，VM 状态仍集中在 Runtime。
    pub fn load_component(&mut self, component: &LuaComponent, id: &Path) -> mlua::Result<()> {
        self.assert_owner_thread();
        if !component.enabled {
            return Ok(());
        }
        if !component.source_override.trim().is_empty() {
            return self.load_script_source_with_components(
                id,
                &component.source_override,
                &component.editor_script,
                &component.components,
                None,
            );
        }
        let resource = component.script.as_ref().ok_or_else(|| {
            mlua::Error::runtime("LuaComponent 未选择 LuaScript 资源，且 source_override 为空")
        })?;
        let data = resource.data_ref();
        let script = data
            .as_loaded_ref()
            .ok_or_else(|| mlua::Error::runtime("LuaComponent 脚本资源尚未加载"))?;
        self.load_script_source_with_components(
            id,
            &script.source,
            &component.editor_script,
            &component.components,
            None,
        )
    }

    /// 扫描一个真实场景图并加载其中的 LuaComponent。
    ///
    /// LuaComponent 只负责序列化配置，所有实例状态仍由此 Runtime 集中管理；
    /// 因此编辑器在节点上新增或修改组件后，运行时不会使用另一份布局副本。
    pub fn load_scene_components(
        &mut self,
        scene: &fyrox::scene::Scene,
        id_prefix: &Path,
    ) -> mlua::Result<usize> {
        self.assert_owner_thread();
        use fyrox::graph::SceneGraph;

        let mut loaded = 0;
        for (node_handle, node) in scene.graph.pair_iter() {
            for (script_index, component) in node.try_get_scripts::<LuaComponent>().enumerate() {
                if !component.enabled || !component.editor_script.enabled {
                    continue;
                }
                if component.script.is_none() && component.source_override.trim().is_empty() {
                    lua_warn!(
                        "跳过未选择 LuaScript 的 LuaComponent: 节点 {:?}, 脚本索引 {}",
                        node_handle,
                        script_index
                    );
                    continue;
                }
                let id = id_prefix.join(format!(
                    "node_{}_script_{}",
                    node_handle.index(),
                    script_index
                ));
                self.load_component_with_scope(
                    component,
                    &id,
                    Some(HandleToken::from_handle(node_handle)),
                )?;
                loaded += 1;
            }
        }
        Ok(loaded)
    }

    #[inline]
    fn load_component_with_scope(
        &mut self,
        component: &LuaComponent,
        id: &Path,
        scope: Option<HandleToken>,
    ) -> mlua::Result<()> {
        self.assert_owner_thread();
        if !component.enabled {
            return Ok(());
        }
        if !component.source_override.trim().is_empty() {
            return self.load_script_source_with_components(
                id,
                &component.source_override,
                &component.editor_script,
                &component.components,
                scope,
            );
        }
        let resource = component.script.as_ref().ok_or_else(|| {
            mlua::Error::runtime("LuaComponent 未选择 LuaScript 资源，且 source_override 为空")
        })?;
        let data = resource.data_ref();
        let script = data
            .as_loaded_ref()
            .ok_or_else(|| mlua::Error::runtime("LuaComponent 脚本资源尚未加载"))?;
        self.load_script_source_with_components(
            id,
            &script.source,
            &component.editor_script,
            &component.components,
            scope,
        )
    }

    /// 返回当前 VM 中已实例化的脚本数量，供宿主状态面板和自检使用。
    pub fn script_count(&self) -> usize {
        self.assert_owner_thread();
        self.scripts.len()
    }
    pub fn call_all(&mut self, name: &str, dt: f32) -> mlua::Result<()> {
        self.assert_owner_thread();
        if self.state != RuntimeState::Running {
            return Ok(());
        }
        if name == "update" {
            if let Some(main) = self.main.as_ref() {
                self.call_main_dt(main.update.as_ref(), main, dt)?;
            }
        }
        if name == "update" {
            for &index in &self.update_indices {
                if let Some(entry) = self.scripts.get(index) {
                    self.call_dt(entry, entry.update.as_ref(), dt)?;
                }
            }
        }
        if name == "update" {
            self.update_ticks += 1;
            if self.update_ticks == 1 || self.update_ticks % 300 == 0 {}
        }
        Ok(())
    }
    pub fn start(&mut self) -> mlua::Result<()> {
        self.assert_owner_thread();
        if self.state != RuntimeState::Created {
            return Ok(());
        }
        self.state = RuntimeState::Started;
        let result = (|| {
            if let Some(main) = self.main.as_ref() {
                self.call_main_no_arg(main.start.as_ref(), main)
                    .map_err(|error| {
                        mlua::Error::runtime(format!("main.lua phase=start error={error}"))
                    })?;
            }
            self.call_no_args("start")
        })();
        if let Err(error) = result {
            let _ = self.destroy();
            return Err(error);
        }
        self.state = RuntimeState::Running;
        Ok(())
    }
    pub fn destroy(&mut self) -> mlua::Result<()> {
        self.assert_owner_thread();
        if matches!(self.state, RuntimeState::Stopping | RuntimeState::Stopped) {
            return Ok(());
        }
        self.state = RuntimeState::Stopping;
        let mut first_error = None;
        for entry in std::mem::take(&mut self.scripts).into_iter().rev() {
            if let Err(error) = self
                .call_no_arg(&entry, entry.destroy.as_ref())
                .map_err(|error| {
                    mlua::Error::runtime(format!(
                        "script id={} path={} phase=on_destroy error={error}",
                        entry.id,
                        entry.path.display()
                    ))
                })
            {
                first_error.get_or_insert(error);
            }
            if let Err(error) = self.remove_script_instance(&entry.path) {
                first_error.get_or_insert(error);
            }
            if let Err(error) = self.remove_instance_keys(entry) {
                first_error.get_or_insert(error);
            }
        }
        self.update_indices.clear();
        if let Some(main) = self.main.take() {
            let result = self
                .call_main_no_arg(main.on_destroy.as_ref(), &main)
                .map_err(|error| {
                    mlua::Error::runtime(format!("main.lua phase=on_destroy error={error}"))
                });
            let cleanup = self.remove_main_keys(main);
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
            if let Err(error) = cleanup {
                first_error.get_or_insert(error);
            }
        };
        for (_, key) in self.scope_tables.get_mut().drain() {
            if let Err(error) = self.lua.remove_registry_value(key) {
                first_error.get_or_insert(error);
            }
        }
        self.state = RuntimeState::Stopped;
        first_error.map_or(Ok(()), Err)
    }
    fn call_no_args(&self, name: &str) -> mlua::Result<()> {
        for entry in &self.scripts {
            let key = match name {
                "start" => entry.start.as_ref(),
                "on_destroy" => entry.destroy.as_ref(),
                _ => None,
            };
            self.call_no_arg(entry, key).map_err(|error| {
                mlua::Error::runtime(format!(
                    "script id={} path={} phase={} error={error}",
                    entry.id,
                    entry.path.display(),
                    name
                ))
            })?;
        }
        Ok(())
    }
    pub fn dispatch_script_event(&mut self, name: &str, payload: &str) -> mlua::Result<()> {
        self.assert_owner_thread();
        if self.state != RuntimeState::Running {
            return Ok(());
        }
        lua_info!(
            "[Lua] event dispatch: name={}, payload_len={}, scripts={}",
            name,
            payload.len(),
            self.scripts.len()
        );
        for entry in &self.scripts {
            self.call_event(entry, entry.event.as_ref(), name, payload)?;
        }
        self.events_dispatched += 1;
        Ok(())
    }

    /// Dispatches a Fyrox UI click to the callback registered by Lua.
    pub fn dispatch_ui_click(&mut self, id: &str) -> mlua::Result<bool> {
        self.assert_owner_thread();
        if self.state != RuntimeState::Running {
            return Ok(false);
        }
        let callbacks: Table = match self.lua.globals().get("__fwok_ui_clicks") {
            Ok(table) => table,
            Err(_) => return Ok(false),
        };
        let value: mlua::Value = match callbacks.get(id) {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let (callback, scope) = match value {
            mlua::Value::Function(callback) => (callback, None),
            mlua::Value::Table(entry) => {
                let callback: Function = entry
                    .get("callback")
                    .map_err(|_| mlua::Error::runtime("invalid Lua UI callback entry"))?;
                let scope = match entry.get::<mlua::Value>("scope")? {
                    mlua::Value::Table(table) => Some(HandleToken {
                        index: table.get("index")?,
                        generation: table.get("generation")?,
                    }),
                    _ => None,
                };
                (callback, scope)
            }
            _ => return Ok(false),
        };
        self.with_scope(scope, || callback.call::<()>(()))?;
        Ok(true)
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use crate::{
        game_api::{Bridge, SceneCommand},
        resource::{EditorScript, LuaComponent},
        Api,
    };
    use fyrox::{
        core::pool::Handle,
        graph::SceneGraph,
        scene::{base::BaseBuilder, node::Node, pivot::PivotBuilder, Scene},
    };
    use std::{cell::RefCell, rc::Rc};

    fn test_api() -> Api {
        Api {
            bridge: Rc::new(RefCell::new(Bridge::default())),
        }
    }

    #[test]
    fn main_scripts_table_tracks_script_lifecycle() {
        let root = std::env::temp_dir().join(format!("lua-main-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("main.lua"),
            r#"local Main = {}; Main.__index = Main
function Main.new(class) return setmetatable({}, class) end
function Main:start()
    local script = self.scripts["tracked.lua"]
    _G.main_script_available = script ~= nil and script.value == 42
end
function Main:on_destroy()
    _G.main_scripts_empty = next(self.scripts) == nil
end
return Main"#,
        )
        .unwrap();
        let bridge = Rc::new(RefCell::new(Bridge::default()));
        let mut config = LuaConfig::default();
        config.script_root = root.clone();
        let mut runtime = LuaRuntime::new_for_scene(config, Api { bridge }).unwrap();
        runtime
            .load_script_source(
                Path::new("tracked.lua"),
                r#"local Script = {}; Script.__index = Script
function Script.new(class) return setmetatable({ value = 42 }, class) end
return Script"#,
                &EditorScript::default(),
            )
            .unwrap();
        runtime.start().unwrap();
        assert!(runtime
            .lua
            .globals()
            .get::<bool>("main_script_available")
            .unwrap());
        runtime.destroy().unwrap();
        assert!(runtime
            .lua
            .globals()
            .get::<bool>("main_scripts_empty")
            .unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scene_component_find_uses_mount_scope() {
        let mut scene = Scene::new();
        let root: Handle<Node> = scene
            .graph
            .add_node(PivotBuilder::new(BaseBuilder::new().with_name("Mount")).build_node());
        let child = scene
            .graph
            .add_node(PivotBuilder::new(BaseBuilder::new().with_name("Child")).build_node());
        scene.graph.link_nodes(child, root);
        scene.graph[root].add_script(LuaComponent {
            script: None,
            source_override: r#"local C={}; C.__index=C
function C.new(class) scene.find("Child"); scene.global_find("Mount"); return setmetatable({}, class) end
function C:on_awake() ui.find("button"):on_click(function() scene.find("Child") end) end
return C"#
                .into(),
            enabled: true,
            editor_script: EditorScript::default(),
            components: Default::default(),
        });
        let bridge = Rc::new(RefCell::new(Bridge::default()));
        let mut runtime = LuaRuntime::new_for_scene(
            LuaConfig::default(),
            Api {
                bridge: bridge.clone(),
            },
        )
        .unwrap();
        runtime
            .load_scene_components(&scene, Path::new("scope"))
            .unwrap();
        runtime.start().unwrap();
        assert!(runtime.dispatch_ui_click("button").unwrap());
        let commands = bridge.borrow();
        assert!(commands.scene_commands.iter().any(|command| matches!(command,
            SceneCommand::Resolve(target) if target.name == "Child" && target.scope == Some(HandleToken::from_handle(root))
        )));
        assert!(commands
            .scene_commands
            .iter()
            .any(|command| matches!(command,
                SceneCommand::Resolve(target) if target.name == "Mount" && target.scope.is_none()
            )));
    }

    #[test]
    fn start_failure_stops_runtime_and_destroys_every_script() {
        let mut runtime = LuaRuntime::new_for_scene(LuaConfig::default(), test_api()).unwrap();
        for (path, source) in [
            (
                "first.lua",
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:start() error("start failed") end
function C:on_destroy() _G.first_destroyed = true end
return C"#,
            ),
            (
                "second.lua",
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:on_destroy() _G.second_destroyed = true end
return C"#,
            ),
        ] {
            runtime
                .load_script_source(Path::new(path), source, &EditorScript::default())
                .unwrap();
        }
        let error = runtime.start().unwrap_err().to_string();
        assert!(error.contains("first.lua"));
        assert!(error.contains("phase=start"));
        assert_eq!(runtime.state, RuntimeState::Stopped);
        assert_eq!(runtime.script_count(), 0);
        assert!(runtime
            .lua
            .globals()
            .get::<bool>("first_destroyed")
            .unwrap());
        assert!(runtime
            .lua
            .globals()
            .get::<bool>("second_destroyed")
            .unwrap());
        runtime.destroy().unwrap();
    }

    #[test]
    fn destroy_error_does_not_skip_other_scripts() {
        let mut runtime = LuaRuntime::new_for_scene(LuaConfig::default(), test_api()).unwrap();
        for (path, source) in [
            (
                "first.lua",
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:on_destroy() _G.first_destroyed = true end
return C"#,
            ),
            (
                "second.lua",
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:on_destroy() error("destroy failed") end
return C"#,
            ),
        ] {
            runtime
                .load_script_source(Path::new(path), source, &EditorScript::default())
                .unwrap();
        }
        runtime.start().unwrap();
        assert!(runtime
            .destroy()
            .unwrap_err()
            .to_string()
            .contains("second.lua"));
        assert!(runtime
            .lua
            .globals()
            .get::<bool>("first_destroyed")
            .unwrap());
        assert_eq!(runtime.script_count(), 0);
        assert_eq!(runtime.state, RuntimeState::Stopped);
    }

    #[test]
    fn nested_scopes_restore_outer_handle() {
        let runtime = LuaRuntime::new_for_scene(LuaConfig::default(), test_api()).unwrap();
        let outer = HandleToken {
            index: 1,
            generation: 2,
        };
        let inner = HandleToken {
            index: 3,
            generation: 4,
        };
        runtime
            .with_scope(Some(outer), || {
                runtime.with_scope(Some(inner), || {
                    let current: Table = runtime.lua.globals().get("__fwok_current_scene_scope")?;
                    assert_eq!(current.get::<u32>("index")?, 3);
                    Ok(())
                })?;
                let current: Table = runtime.lua.globals().get("__fwok_current_scene_scope")?;
                assert_eq!(current.get::<u32>("index")?, 1);
                Ok(())
            })
            .unwrap();
    }
}
