use crate::handles::HandleToken;
use crate::{
    bindings::{register_engine_bindings, register_generated_component_aliases},
    config::{BindingMode, LuaConfig, LuaErrorPolicy},
    diagnostics::{bounded_message, ErrorBuffer},
    resource::{EditorScript, LuaComponent},
    LuaGameApi,
};
use fyrox::core::instant::Instant;
use mlua::{Function, Lua, RegistryKey, Table, Value};
#[cfg(target_arch = "wasm32")]
use mlua::{LuaOptions, StdLib};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

/// Host supplied Lua source loader.
///
/// The plugin only knows how to execute Lua source.  A game or another host
/// decides where that source comes from (the filesystem, Fyrox's resource
/// system, a package table, or a platform asset API).  Keeping this callback
/// outside `LuaConfig` also keeps serialized project configuration free of
/// executable state.
pub type LuaSourceLoader = Rc<dyn Fn(&Path) -> mlua::Result<String>>;

fn filesystem_source_loader(path: &Path) -> mlua::Result<String> {
    fs::read_to_string(path).map_err(mlua::Error::external)
}

fn module_path(root: &Path, module: &str) -> mlua::Result<PathBuf> {
    if module.is_empty() {
        return Err(mlua::Error::runtime("Lua module name must not be empty"));
    }
    let mut relative = PathBuf::new();
    for component in module.split('.') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.contains(['/', '\\'])
        {
            return Err(mlua::Error::runtime(format!(
                "invalid Lua module name: {module}"
            )));
        }
        relative.push(component);
    }
    relative.set_extension("lua");
    Ok(root.join(relative))
}

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

pub use crate::diagnostics::LuaErrorReport;

pub struct LuaRuntime {
    pub lua: Lua,
    scripts: Vec<ScriptInstance>,
    update_indices: Vec<usize>,
    main: Option<MainScript>,
    config: LuaConfig,
    state: RuntimeState,
    next_script_id: u64,
    scope_tables: RefCell<HashMap<HandleToken, RegistryKey>>,
    errors: ErrorBuffer,
    disabled_scripts: HashSet<u64>,
    script_error_counts: HashMap<u64, u32>,
    main_error_count: u32,
    main_disabled: bool,
    update_ticks: u64,
    events_dispatched: u64,
    source_loader: LuaSourceLoader,
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
    registered_in_main: bool,
}
struct MainScript {
    path: PathBuf,
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
        Self::new_inner(config, api, Rc::new(filesystem_source_loader))
    }

    /// Creates a runtime with source loading owned by the host.
    ///
    /// This is the browser and embedded-platform entry point.  The plugin
    /// never embeds or enumerates project scripts; the callback is supplied by
    /// the game/resource host instead.
    pub fn new_with_source_loader(
        config: LuaConfig,
        api: impl LuaGameApi,
        source_loader: LuaSourceLoader,
    ) -> mlua::Result<Self> {
        Self::new_inner(config, api, source_loader)
    }

    /// Creates a runtime that combines project scripts (including `main.lua`) with
    /// scene-owned LuaComponent instances in one VM. Root class scripts are loaded
    /// by `LuaPluginHost::start_scene`.
    pub fn new_for_scene(config: LuaConfig, api: impl LuaGameApi) -> mlua::Result<Self> {
        Self::new_inner(config, api, Rc::new(filesystem_source_loader))
    }

    pub fn new_for_scene_with_source_loader(
        config: LuaConfig,
        api: impl LuaGameApi,
        source_loader: LuaSourceLoader,
    ) -> mlua::Result<Self> {
        Self::new_inner(config, api, source_loader)
    }

    fn new_inner(
        config: LuaConfig,
        api: impl LuaGameApi,
        source_loader: LuaSourceLoader,
    ) -> mlua::Result<Self> {
        let init_started = Instant::now();
        let root = config.script_root.clone();
        lua_info!(
            "[Lua] runtime initializing (root={}, binding={:?})",
            root.display(),
            config.effective_binding_mode()
        );
        #[cfg(target_arch = "wasm32")]
        let lua = Lua::new_with(
            StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH,
            LuaOptions::default(),
        )?;
        #[cfg(not(target_arch = "wasm32"))]
        let lua = Lua::new();
        // 让场景脚本可以通过标准 require 互相组合；根目录由配置决定。
        if let Ok(package) = lua.globals().get::<Table>("package") {
            let path = format!("{}/?.lua;{}/?/init.lua", root.display(), root.display());
            package.set("path", path)?;
        }
        // `import` is a small, host-backed module loader.  It deliberately
        // accepts only module names and resolves them below the configured
        // script root; source bytes and the module list belong to the host.
        let loaded = lua.create_table()?;
        lua.globals().set("__fwok_loaded_modules", loaded)?;
        let import_root = root.clone();
        let import_loader = source_loader.clone();
        let import = lua.create_function(move |lua, module: String| {
            let loaded: Table = lua.globals().get("__fwok_loaded_modules")?;
            if let Some(value) = loaded.raw_get::<Option<Value>>(module.as_str())? {
                return Ok(value);
            }
            let path = module_path(&import_root, &module)?;
            let source = import_loader(&path)?;
            let value = lua
                .load(&source)
                .set_name(format!("@{}", path.display()))
                .eval::<Value>()?;
            let value = if matches!(value, Value::Nil) {
                Value::Boolean(true)
            } else {
                value
            };
            loaded.raw_set(module, value.clone())?;
            Ok(value)
        })?;
        lua.globals().set("import", import)?;
        // 引擎绑定只从手写注册入口装载；这里绝不扫描 Lua 源码。
        register_engine_bindings(&lua)?;
        lua_info!(
            "[Lua] API registration complete (mode={:?}, executable bindings)",
            config.binding_mode
        );
        api.register(&lua)?;
        register_generated_component_aliases(&lua)?;
        // Pure-Lua helpers are part of the plugin artifact and never depend on
        // project/business files.  Loading them once here keeps hot paths in
        // Lua and avoids a Rust callback for common math/table/string work.
        crate::embedded::register(&lua)?;
        let mut runtime = Self {
            lua,
            scripts: vec![],
            update_indices: vec![],
            main: None,
            config,
            state: RuntimeState::Created,
            next_script_id: 1,
            scope_tables: RefCell::new(HashMap::new()),
            errors: ErrorBuffer::default(),
            disabled_scripts: HashSet::new(),
            script_error_counts: HashMap::new(),
            main_error_count: 0,
            main_disabled: false,
            update_ticks: 0,
            events_dispatched: 0,
            source_loader,
            _main_thread_only: Rc::new(()),
        };
        if let Err(error) = runtime.load_main(&root) {
            lua_warn!(
                "[LuaPerf] lua_init_ms={:.3} status=error phase=main_load root={} error={}",
                init_started.elapsed().as_secs_f64() * 1000.0,
                root.display(),
                error
            );
            return Err(error);
        }
        if runtime.state == RuntimeState::Stopping {
            return Err(mlua::Error::runtime(
                "Lua runtime stopped during main.lua initialization",
            ));
        }
        lua_info!(
            "[Lua] runtime initialized (main={}, component_instances={})",
            runtime.main.is_some(),
            runtime.scripts.len()
        );
        lua_info!(
            "[LuaPerf] lua_init_ms={:.3} root={} main={} component_instances={}",
            init_started.elapsed().as_secs_f64() * 1000.0,
            root.display(),
            runtime.main.is_some(),
            runtime.scripts.len()
        );
        Ok(runtime)
    }
    pub fn binding_mode(&self) -> BindingMode {
        self.config.effective_binding_mode()
    }

    pub fn is_running(&self) -> bool {
        self.state == RuntimeState::Running
    }

    pub fn has_main_script(&self) -> bool {
        self.main.is_some()
    }
    pub fn config(&self) -> &LuaConfig {
        &self.config
    }

    pub fn reload_script(&mut self, path: &Path) -> mlua::Result<()> {
        let register_in_main = if let Some(i) =
            self.scripts.iter().position(|entry| entry.path == path)
        {
            let old = self.scripts.remove(i);
            if old.registered_in_main {
                self.scripts.insert(i, old);
                return Err(mlua::Error::runtime(format!(
                        "cannot reload LuaComponent instance '{}' through load_script; reload the component resource instead",
                        path.display()
                    )));
            }
            let register_in_main = old.registered_in_main;
            let old_path = old.path.clone();
            self.clear_ui_callbacks_for_scope(old.scope)?;
            let destroy_result = self.call_no_arg(&old, old.destroy.as_ref());
            if register_in_main {
                self.remove_script_instance(&old.path)?;
            }
            self.remove_instance_keys(old)?;
            self.rebuild_update_indices();
            if let Err(error) = destroy_result {
                self.report_error("reload.on_destroy", Some(&old_path), error);
            }
            register_in_main
        } else {
            false
        };
        self.load_script_with_registration(path, register_in_main)
    }
    /// Loads the optional project entry point. `main.lua` returns a table whose
    /// dot-style lifecycle functions are called in the same VM as component scripts.
    fn load_main(&mut self, root: &Path) -> mlua::Result<()> {
        let path = root.join("main.lua");
        let load_started = Instant::now();
        let source = match self.read_source(&path) {
            Ok(source) => source,
            Err(mlua::Error::ExternalError(error))
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                lua_info!(
                    "[LuaPerf] main_load_ms=0.000 main_awake_ms=0.000 status=absent path={}",
                    path.display()
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
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
            path: path.clone(),
            on_awake: Self::method_key(&self.lua, &instance, "on_awake")?,
            start: Self::method_key(&self.lua, &instance, "start")?,
            update: Self::method_key(&self.lua, &instance, "update")?,
            on_destroy: Self::method_key(&self.lua, &instance, "on_destroy")?,
            scripts: self.lua.create_registry_value(scripts)?,
            instance: self.lua.create_registry_value(instance)?,
        };
        let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
        let awake_started = Instant::now();
        let mut awake_status = "ok";
        if let Err(error) = self.call_main_no_arg(entry.on_awake.as_ref(), &entry) {
            awake_status = "error_recovered";
            let disable = self.should_disable_main();
            self.report_error("main.on_awake", Some(&path), error);
            if self.config.error_policy == LuaErrorPolicy::StopRuntime {
                self.state = RuntimeState::Stopping;
            }
            self.main_disabled = disable;
        }
        lua_info!(
            "[LuaPerf] main_load_ms={:.3} main_awake_ms={:.3} main_awake_status={} path={}",
            load_ms,
            awake_started.elapsed().as_secs_f64() * 1000.0,
            awake_status,
            path.display()
        );
        self.main = Some(entry);
        lua_info!("[Lua] main.lua loaded: {}", path.display());
        Ok(())
    }
    pub fn load_script(&mut self, p: &Path) -> mlua::Result<()> {
        self.load_script_with_registration(p, false)
    }

    fn load_script_with_registration(
        &mut self,
        p: &Path,
        register_in_main: bool,
    ) -> mlua::Result<()> {
        let source = self.read_source(p)?;
        self.load_script_source_with_components(
            p,
            &source,
            &EditorScript::default(),
            &Default::default(),
            None,
            register_in_main,
        )
    }

    /// Explicitly instantiates a script without adding it to `main.scripts`.
    ///
    /// A script becomes visible through `main.scripts` only when instantiated by
    /// [`Self::load_component`] or [`Self::load_scene_components`].
    pub fn load_script_source(
        &mut self,
        id: &Path,
        source: &str,
        editor_script: &EditorScript,
    ) -> mlua::Result<()> {
        self.load_script_source_with_components(
            id,
            source,
            editor_script,
            &Default::default(),
            None,
            false,
        )
    }

    fn load_script_source_with_components(
        &mut self,
        id: &Path,
        source: &str,
        editor_script: &EditorScript,
        component_bindings: &crate::component::ComponentBindings,
        scope: Option<HandleToken>,
        register_in_main: bool,
    ) -> mlua::Result<()> {
        self.check_source_size(id, source.len())?;
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
            registered_in_main: register_in_main,
            id: self.next_script_id,
        };
        self.next_script_id = self.next_script_id.saturating_add(1);
        let has_update = entry.update.is_some();
        if register_in_main {
            self.register_script_instance(id, &self.lua.registry_value(&entry.instance)?)?;
        }
        if let Err(error) = self.call_no_arg(&entry, entry.awake.as_ref()) {
            let disable = self.should_disable_script(entry.id);
            self.report_script_error("on_awake", &entry, error);
            if disable {
                self.disabled_scripts.insert(entry.id);
            }
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

    #[inline]
    fn check_source_size(&self, path: &Path, bytes: usize) -> mlua::Result<()> {
        if bytes > self.config.max_script_bytes {
            return Err(mlua::Error::runtime(format!(
                "Lua script '{}' exceeds max_script_bytes={} (actual={})",
                path.display(),
                self.config.max_script_bytes,
                bytes
            )));
        }
        Ok(())
    }

    fn read_source(&self, path: &Path) -> mlua::Result<String> {
        let source = (self.source_loader)(path)?;
        self.check_source_size(path, source.len())?;
        Ok(source)
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
    }

    fn call_main_no_arg(&self, key: Option<&RegistryKey>, entry: &MainScript) -> mlua::Result<()> {
        let Some(key) = key else { return Ok(()) };
        let instance: Table = self.lua.registry_value(&entry.instance)?;
        let function: Function = self.lua.registry_value(key)?;
        function.call::<()>(instance)
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
        function.call::<()>((instance, dt))
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
                true,
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
            true,
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
        let load_started = Instant::now();
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
                match self.load_component_with_scope(
                    component,
                    &id,
                    Some(HandleToken::from_handle(node_handle)),
                ) {
                    Ok(()) => loaded += 1,
                    Err(error) => self.report_error("load", Some(&id), error),
                }
            }
        }
        lua_info!(
            "[LuaPerf] scene_components_load_ms={:.3} loaded={} path={}",
            load_started.elapsed().as_secs_f64() * 1000.0,
            loaded,
            id_prefix.display()
        );
        Ok(loaded)
    }

    #[inline]
    fn load_component_with_scope(
        &mut self,
        component: &LuaComponent,
        id: &Path,
        scope: Option<HandleToken>,
    ) -> mlua::Result<()> {
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
                true,
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
            true,
        )
    }

    /// 返回当前 VM 中已实例化的脚本数量，供宿主状态面板和自检使用。
    ///
    /// This is the component count kept for compatibility with callers that
    /// use the value returned by `start_scene`. The optional `main.lua` entry
    /// point is reported separately by [`Self::loaded_script_count`].
    pub fn script_count(&self) -> usize {
        self.scripts.len()
    }

    /// Returns the number of loaded Lua script instances in this VM.
    ///
    /// `main.lua` is a lifecycle owner and is not stored in `scripts`, so a
    /// component-only count can otherwise report zero while the main entry
    /// point is loaded. Runtime diagnostics and memory samples use this total
    /// to make their script count unambiguous.
    #[inline]
    pub fn loaded_script_count(&self) -> usize {
        self.scripts.len() + usize::from(self.main.is_some())
    }

    /// Number of update dispatches completed by this runtime.
    pub fn update_ticks(&self) -> u64 {
        self.update_ticks
    }

    /// Drains callback diagnostics collected since the previous call.
    pub fn take_errors(&mut self) -> Vec<LuaErrorReport> {
        self.errors.drain()
    }

    fn report_error(
        &mut self,
        phase: impl Into<String>,
        script: Option<&Path>,
        error: impl std::fmt::Display,
    ) {
        self.report_error_with_context(phase, script, None, None, error);
    }

    fn report_script_error(
        &mut self,
        phase: impl Into<String>,
        entry: &ScriptInstance,
        error: impl std::fmt::Display,
    ) {
        self.report_error_with_context(
            phase,
            Some(&entry.path),
            Some(entry.id),
            entry.scope,
            error,
        );
    }

    fn report_error_with_context(
        &mut self,
        phase: impl Into<String>,
        script: Option<&Path>,
        instance_id: Option<u64>,
        scope: Option<HandleToken>,
        error: impl std::fmt::Display,
    ) {
        let report = LuaErrorReport {
            phase: phase.into(),
            script: script.map(Path::to_path_buf),
            instance_id,
            scope,
            frame: self.update_ticks,
            message: bounded_message(error),
            recovered: true,
        };
        lua_warn!(
            "[Lua] recovered callback error: phase={}, script={}, error={}",
            report.phase,
            report
                .script
                .as_deref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<main>".to_owned()),
            report.message
        );
        self.errors.push(report);
    }

    fn should_disable_script(&mut self, id: u64) -> bool {
        let count = self.script_error_counts.entry(id).or_default();
        *count = count.saturating_add(1);
        match self.config.error_policy {
            LuaErrorPolicy::StopRuntime => true,
            LuaErrorPolicy::DisableScript => true,
            LuaErrorPolicy::LogAndContinue => *count >= self.config.max_errors_per_script.max(1),
        }
    }

    fn should_disable_main(&mut self) -> bool {
        self.main_error_count = self.main_error_count.saturating_add(1);
        match self.config.error_policy {
            LuaErrorPolicy::StopRuntime | LuaErrorPolicy::DisableScript => true,
            LuaErrorPolicy::LogAndContinue => {
                self.main_error_count >= self.config.max_errors_per_script.max(1)
            }
        }
    }

    pub fn call_all(&mut self, name: &str, dt: f32) -> mlua::Result<()> {
        if self.state != RuntimeState::Running {
            return Ok(());
        }
        if name == "update" {
            if !self.main_disabled {
                if let Some(main) = self.main.as_ref() {
                    let result = self.call_main_dt(main.update.as_ref(), main, dt);
                    if let Err(error) = result {
                        let path = main.path.clone();
                        let disable = self.should_disable_main();
                        if self.config.error_policy == LuaErrorPolicy::StopRuntime {
                            self.state = RuntimeState::Stopping;
                        }
                        self.main_disabled = disable;
                        self.report_error("main.update", Some(&path), error);
                        if self.state == RuntimeState::Stopping {
                            return Ok(());
                        }
                    }
                }
            }
        }
        if name == "update" {
            for position in 0..self.update_indices.len() {
                let index = self.update_indices[position];
                let Some((id, scope, result)) = self.scripts.get(index).and_then(|entry| {
                    if self.disabled_scripts.contains(&entry.id) {
                        return None;
                    }
                    Some((
                        entry.id,
                        entry.scope,
                        self.call_dt(entry, entry.update.as_ref(), dt),
                    ))
                }) else {
                    continue;
                };
                if let Err(error) = result {
                    let path = self.scripts[index].path.clone();
                    let disable = self.should_disable_script(id);
                    if self.config.error_policy == LuaErrorPolicy::StopRuntime {
                        self.state = RuntimeState::Stopping;
                    }
                    if disable {
                        self.disabled_scripts.insert(id);
                    }
                    self.report_error_with_context("update", Some(&path), Some(id), scope, error);
                    if self.state == RuntimeState::Stopping {
                        break;
                    }
                }
            }
        }
        if name == "update" {
            self.update_ticks += 1;
        }
        Ok(())
    }
    pub fn start(&mut self) -> mlua::Result<()> {
        if self.state != RuntimeState::Created {
            return Ok(());
        }
        self.state = RuntimeState::Started;
        let start_started = Instant::now();
        let main_start_started = Instant::now();
        let mut main_start_status = "absent";
        if !self.main_disabled {
            if let Some(main) = self.main.as_ref() {
                main_start_status = "ok";
                if let Err(error) = self.call_main_no_arg(main.start.as_ref(), main) {
                    main_start_status = "error_recovered";
                    let path = main.path.clone();
                    let disable = self.should_disable_main();
                    if self.config.error_policy == LuaErrorPolicy::StopRuntime {
                        self.state = RuntimeState::Stopping;
                    }
                    self.main_disabled = disable;
                    self.report_error("main.start", Some(&path), error);
                }
            }
        }
        lua_info!(
            "[LuaPerf] main_start_ms={:.3} main_start_status={}",
            main_start_started.elapsed().as_secs_f64() * 1000.0,
            main_start_status
        );
        if self.state == RuntimeState::Started {
            self.call_no_args("start");
        }
        if self.state == RuntimeState::Stopping {
            lua_info!(
                "[LuaPerf] lua_start_ms={:.3} loaded_scripts={} component_scripts={} status=stopping",
                start_started.elapsed().as_secs_f64() * 1000.0,
                self.loaded_script_count(),
                self.scripts.len()
            );
            return Ok(());
        }
        self.state = RuntimeState::Running;
        lua_info!(
            "[LuaPerf] lua_start_ms={:.3} loaded_scripts={} component_scripts={}",
            start_started.elapsed().as_secs_f64() * 1000.0,
            self.loaded_script_count(),
            self.scripts.len()
        );
        Ok(())
    }
    pub fn destroy(&mut self) -> mlua::Result<()> {
        if self.state == RuntimeState::Stopped {
            return Ok(());
        }
        self.state = RuntimeState::Stopping;
        for entry in std::mem::take(&mut self.scripts).into_iter().rev() {
            if let Err(error) = self.call_no_arg(&entry, entry.destroy.as_ref()) {
                self.report_error("on_destroy", Some(&entry.path), error);
            }
            if entry.registered_in_main {
                if let Err(error) = self.remove_script_instance(&entry.path) {
                    self.report_error("cleanup", Some(&entry.path), error);
                }
            }
            if let Err(error) = self.remove_instance_keys(entry) {
                self.report_error("cleanup", None, error);
            }
        }
        self.update_indices.clear();
        if let Some(main) = self.main.take() {
            let path = main.path.clone();
            let result = self.call_main_no_arg(main.on_destroy.as_ref(), &main);
            let cleanup = self.remove_main_keys(main);
            if let Err(error) = result {
                self.report_error("main.on_destroy", Some(&path), error);
            }
            if let Err(error) = cleanup {
                self.report_error("cleanup", None, error);
            }
        };
        let scope_keys: Vec<_> = self
            .scope_tables
            .get_mut()
            .drain()
            .map(|(_, key)| key)
            .collect();
        for key in scope_keys {
            if let Err(error) = self.lua.remove_registry_value(key) {
                self.report_error("cleanup", None, error);
            }
        }
        if let Err(error) = self.clear_ui_callbacks() {
            self.report_error("cleanup.ui_callbacks", None, error);
        }
        self.state = RuntimeState::Stopped;
        Ok(())
    }

    /// Removes callbacks owned by a scene component before its registry keys
    /// are released.  tolua's delegate maps perform the same owner-aware
    /// cleanup so reloads do not retain old Lua instances through events.
    fn clear_ui_callbacks_for_scope(&self, scope: Option<HandleToken>) -> mlua::Result<()> {
        let Some(scope) = scope else {
            return Ok(());
        };
        let Ok(callbacks) = self.lua.globals().get::<Table>("__fwok_ui_clicks") else {
            return Ok(());
        };
        let mut remove = Vec::new();
        for pair in callbacks.pairs::<String, Value>() {
            let (id, value) = pair?;
            let Value::Table(entry) = value else { continue };
            let Value::Table(owner) = entry.get("scope")? else {
                continue;
            };
            if owner.get::<u32>("index")? == scope.index
                && owner.get::<u32>("generation")? == scope.generation
            {
                remove.push(id);
            }
        }
        for id in remove {
            callbacks.set(id, Value::Nil)?;
        }
        Ok(())
    }

    fn clear_ui_callbacks(&self) -> mlua::Result<()> {
        let Ok(callbacks) = self.lua.globals().get::<Table>("__fwok_ui_clicks") else {
            return Ok(());
        };
        let keys = callbacks
            .pairs::<Value, Value>()
            .map(|pair| pair.map(|(key, _)| key))
            .collect::<mlua::Result<Vec<_>>>()?;
        for key in keys {
            callbacks.set(key, Value::Nil)?;
        }
        Ok(())
    }
    fn call_no_args(&mut self, name: &str) {
        for index in 0..self.scripts.len() {
            let (id, path, scope, result) = {
                let entry = &self.scripts[index];
                (
                    entry.id,
                    entry.path.clone(),
                    entry.scope,
                    self.call_no_arg(
                        entry,
                        match name {
                            "start" => entry.start.as_ref(),
                            "on_destroy" => entry.destroy.as_ref(),
                            _ => None,
                        },
                    ),
                )
            };
            if let Err(error) = result {
                if name != "on_destroy" {
                    if self.config.error_policy == LuaErrorPolicy::StopRuntime {
                        self.state = RuntimeState::Stopping;
                    }
                    if self.should_disable_script(id) {
                        self.disabled_scripts.insert(id);
                    }
                }
                self.report_error_with_context(name, Some(&path), Some(id), scope, error);
                if self.state == RuntimeState::Stopping {
                    break;
                }
            }
        }
    }
    pub fn dispatch_script_event(&mut self, name: &str, payload: &str) -> mlua::Result<()> {
        if self.state != RuntimeState::Running {
            return Ok(());
        }
        if self.events_dispatched == 0 || self.events_dispatched % 300 == 0 {
            lua_info!(
                "[Lua] event dispatch: count={}, name={}, payload_len={}, loaded_scripts={}, component_scripts={}",
                self.events_dispatched + 1,
                name,
                payload.len(),
                self.loaded_script_count(),
                self.scripts.len()
            );
        }
        for index in 0..self.scripts.len() {
            let (id, scope, result) = {
                let entry = &self.scripts[index];
                if self.disabled_scripts.contains(&entry.id) {
                    continue;
                }
                (
                    entry.id,
                    entry.scope,
                    self.call_event(entry, entry.event.as_ref(), name, payload),
                )
            };
            if let Err(error) = result {
                let path = self.scripts[index].path.clone();
                if self.config.error_policy == LuaErrorPolicy::StopRuntime {
                    self.state = RuntimeState::Stopping;
                }
                if self.should_disable_script(id) {
                    self.disabled_scripts.insert(id);
                }
                self.report_error_with_context(
                    format!("event:{name}"),
                    Some(&path),
                    Some(id),
                    scope,
                    error,
                );
                if self.state == RuntimeState::Stopping {
                    break;
                }
            }
        }
        self.events_dispatched += 1;
        Ok(())
    }

    /// Dispatches a Fyrox UI click to the callback registered by Lua.
    pub fn dispatch_ui_click(&mut self, id: &str) -> mlua::Result<bool> {
        if self.state != RuntimeState::Running {
            return Ok(false);
        }
        let callbacks: Table = match self.lua.globals().get("__fwok_ui_clicks") {
            Ok(table) => table,
            Err(error) => {
                self.report_error("ui_click.lookup", None, error);
                return Ok(false);
            }
        };
        let value: mlua::Value = match callbacks.raw_get(id) {
            Ok(value) => value,
            Err(error) => {
                self.report_error(format!("ui_click.lookup:{id}"), None, error);
                return Ok(false);
            }
        };
        let (callback, scope) = match value {
            mlua::Value::Function(callback) => (callback, None),
            mlua::Value::Table(entry) => {
                let callback: Function = match entry.raw_get("callback") {
                    Ok(callback) => callback,
                    Err(error) => {
                        self.report_error(format!("ui_click:{id}"), None, error);
                        return Ok(false);
                    }
                };
                let scope = match entry.raw_get::<mlua::Value>("scope") {
                    Ok(value) => match value {
                        mlua::Value::Table(table) => Some(HandleToken {
                            index: match table.raw_get("index") {
                                Ok(value) => value,
                                Err(error) => {
                                    self.report_error(format!("ui_click:{id}"), None, error);
                                    return Ok(false);
                                }
                            },
                            generation: match table.raw_get("generation") {
                                Ok(value) => value,
                                Err(error) => {
                                    self.report_error(format!("ui_click:{id}"), None, error);
                                    return Ok(false);
                                }
                            },
                        }),
                        _ => None,
                    },
                    Err(error) => {
                        self.report_error(format!("ui_click:{id}"), None, error);
                        return Ok(false);
                    }
                };
                (callback, scope)
            }
            _ => return Ok(false),
        };
        if let Err(error) = self.with_scope(scope, || callback.call::<()>(())) {
            self.report_error(format!("ui_click:{id}"), None, error);
            return Ok(false);
        }
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
            .load_component(
                &LuaComponent {
                    script: None,
                    source_override: r#"local Script = {}; Script.__index = Script
function Script.new(class) return setmetatable({ value = 42 }, class) end
return Script"#
                        .into(),
                    enabled: true,
                    editor_script: EditorScript::default(),
                    components: Default::default(),
                },
                Path::new("tracked.lua"),
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
    fn direct_script_load_is_not_registered_in_main_scripts() {
        let root =
            std::env::temp_dir().join(format!("lua-main-direct-script-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("main.lua"),
            r#"local Main = {}; function Main:start()
    _G.direct_script_registered = self.scripts["direct.lua"] ~= nil
end; return Main"#,
        )
        .unwrap();
        let mut config = LuaConfig::default();
        config.script_root = root.clone();
        let mut runtime = LuaRuntime::new_for_scene(config, test_api()).unwrap();
        runtime
            .load_script_source(
                Path::new("direct.lua"),
                r#"local Script = {}; function Script.new(class) return class end; return Script"#,
                &EditorScript::default(),
            )
            .unwrap();
        runtime.start().unwrap();
        assert!(!runtime
            .lua
            .globals()
            .get::<bool>("direct_script_registered")
            .unwrap());
        runtime.destroy().unwrap();
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
            SceneCommand::Resolve(target) if target.name.as_ref() == "Child" && target.scope == Some(HandleToken::from_handle(root))
        )));
        assert!(commands
            .scene_commands
            .iter()
            .any(|command| matches!(command,
                SceneCommand::Resolve(target) if target.name.as_ref() == "Mount" && target.scope.is_none()
            )));
    }

    #[test]
    fn start_failure_isolated_and_destroy_still_runs_every_script() {
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
        runtime.start().unwrap();
        let errors = runtime.take_errors();
        assert!(errors
            .iter()
            .any(|error| error.script.as_deref() == Some(Path::new("first.lua"))));
        assert_eq!(runtime.state, RuntimeState::Running);
        assert_eq!(runtime.script_count(), 2);
        runtime.destroy().unwrap();
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
    fn update_failure_does_not_stop_other_script_or_next_frame() {
        let mut runtime = LuaRuntime::new_for_scene(LuaConfig::default(), test_api()).unwrap();
        runtime
            .load_script_source(
                Path::new("bad_update.lua"),
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:update() error("update failed") end
return C"#,
                &EditorScript::default(),
            )
            .unwrap();
        runtime
            .load_script_source(
                Path::new("good_update.lua"),
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:update() _G.good_updates = (_G.good_updates or 0) + 1 end
return C"#,
                &EditorScript::default(),
            )
            .unwrap();
        runtime.start().unwrap();
        runtime.call_all("update", 0.016).unwrap();
        runtime.call_all("update", 0.016).unwrap();
        assert_eq!(runtime.lua.globals().get::<u32>("good_updates").unwrap(), 2);
        assert!(runtime
            .take_errors()
            .iter()
            .any(|error| error.script.as_deref() == Some(Path::new("bad_update.lua"))));
    }

    #[test]
    fn log_and_continue_policy_preserves_ml_error_and_structured_context() {
        let mut config = LuaConfig::default();
        config.error_policy = LuaErrorPolicy::LogAndContinue;
        config.max_errors_per_script = 3;
        let mut runtime = LuaRuntime::new_for_scene(config, test_api()).unwrap();
        runtime
            .load_script_source(
                Path::new("policy.lua"),
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:update() error("policy failure") end
return C"#,
                &EditorScript::default(),
            )
            .unwrap();
        runtime.start().unwrap();
        runtime.call_all("update", 0.016).unwrap();
        runtime.call_all("update", 0.016).unwrap();
        let errors = runtime.take_errors();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|error| {
            error.instance_id.is_some()
                && error.phase == "update"
                && error.message.contains("policy failure")
                && error.recovered
        }));
        assert!(errors.iter().any(|error| error.frame > 0));
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
        runtime.destroy().unwrap();
        assert!(runtime
            .take_errors()
            .iter()
            .any(|error| error.script.as_deref() == Some(Path::new("second.lua"))));
        assert!(runtime
            .lua
            .globals()
            .get::<bool>("first_destroyed")
            .unwrap());
        assert_eq!(runtime.script_count(), 0);
        assert_eq!(runtime.state, RuntimeState::Stopped);
    }

    #[test]
    fn destroy_after_stop_runtime_still_releases_scripts() {
        let mut runtime = LuaRuntime::new_for_scene(LuaConfig::default(), test_api()).unwrap();
        runtime
            .load_script_source(
                Path::new("stopping.lua"),
                r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:on_destroy() _G.stopping_destroyed = true end
return C"#,
                &EditorScript::default(),
            )
            .unwrap();
        runtime.start().unwrap();
        runtime.state = RuntimeState::Stopping;
        runtime.destroy().unwrap();
        assert_eq!(runtime.script_count(), 0);
        assert!(runtime
            .lua
            .globals()
            .get::<bool>("stopping_destroyed")
            .unwrap());
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
