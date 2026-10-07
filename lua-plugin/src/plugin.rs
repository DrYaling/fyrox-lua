//! Main-thread Lua plugin host.

use crate::diagnostics::{bounded_message, ErrorBuffer};
use crate::{
    Api, BridgeHandle, BridgeStats, LuaApi, LuaApiRegistry, LuaCommandHost, LuaConfig,
    LuaErrorReport, LuaRuntime, LuaSourceLoader,
};
use fyrox::{
    core::pool::Handle,
    gui::{font::FontResource, UserInterface},
    plugin::PluginContext,
    scene::Scene,
};
use std::{path::Path, rc::Rc};

pub struct LuaPluginHost {
    config: LuaConfig,
    bridge: BridgeHandle,
    runtime: Option<LuaRuntime>,
    api_registry: LuaApiRegistry,
    command_host: Option<LuaCommandHost>,
    errors: ErrorBuffer,
    source_loader: Option<LuaSourceLoader>,
    _main_thread_only: Rc<()>,
}

impl std::fmt::Debug for LuaPluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaPluginHost")
            .field("runtime_active", &self.runtime.is_some())
            .finish()
    }
}

impl LuaPluginHost {
    pub fn new(config: LuaConfig) -> Self {
        Self {
            config,
            bridge: BridgeHandle::default(),
            runtime: None,
            api_registry: LuaApiRegistry::new(),
            command_host: None,
            errors: ErrorBuffer::default(),
            source_loader: None,
            _main_thread_only: Rc::new(()),
        }
    }

    pub fn config(&self) -> &LuaConfig {
        &self.config
    }
    pub fn bridge(&self) -> &BridgeHandle {
        &self.bridge
    }

    /// Installs a host-owned source loader for project Lua resources.
    ///
    /// The loader is intentionally independent of the serialized Lua
    /// configuration.  Browser and embedded hosts can provide package or
    /// resource-manager access without putting project knowledge in the
    /// plugin.
    pub fn set_source_loader(&mut self, loader: LuaSourceLoader) {
        self.source_loader = Some(loader);
    }

    /// Returns a copy of bridge counters without exposing the mutable command buffer.
    pub fn bridge_stats(&self) -> BridgeStats {
        self.bridge.0.borrow().stats()
    }

    /// Attaches the generic UI/Scene command host to an already loaded scene.
    ///
    /// The command host is owned by the plugin so project crates do not need to
    /// carry or apply engine command queues themselves. Calling this again
    /// replaces the previous scene state and keeps the current bridge handle.
    pub fn attach_scene(&mut self, scene: Handle<Scene>) {
        let mut command_host = LuaCommandHost::new(scene);
        command_host.install_bridge(self.bridge.clone());
        self.command_host = Some(command_host);
    }

    pub fn scene(&self) -> Option<Handle<Scene>> {
        self.command_host.as_ref().map(LuaCommandHost::scene)
    }

    pub fn ui_handle(&self) -> Handle<UserInterface> {
        self.command_host
            .as_ref()
            .map(LuaCommandHost::ui_handle)
            .unwrap_or(Handle::NONE)
    }

    pub fn ui_visible(&self) -> bool {
        self.command_host
            .as_ref()
            .map(LuaCommandHost::ui_visible)
            .unwrap_or(true)
    }

    pub fn ui_registry(&self) -> Option<&crate::UiRegistry> {
        self.command_host
            .as_ref()
            .and_then(LuaCommandHost::ui_registry)
    }

    /// Applies UI commands before a Lua update and Scene commands after it.
    /// This split preserves the one-frame ordering used by the runtime while
    /// keeping the implementation entirely in lua-plugin.
    pub fn apply_ui_commands(&mut self, context: &mut PluginContext) -> Vec<String> {
        let Some(command_host) = self.command_host.as_mut() else {
            return Vec::new();
        };
        command_host.apply_ui_commands(context);
        command_host.take_ui_load_requests()
    }

    pub fn apply_scene_commands(&mut self, context: &mut PluginContext) {
        if let Some(command_host) = self.command_host.as_mut() {
            command_host.apply_scene_commands(context);
        }
    }

    /// Runs one frame of Lua and applies all generic bridge commands in the
    /// correct order. The returned paths are project UI resource requests that
    /// the caller should submit to its resource system.
    pub fn update_with_context(
        &mut self,
        context: &mut PluginContext,
        dt: f32,
    ) -> mlua::Result<Vec<String>> {
        let requests = self.apply_ui_commands(context);
        let result = self.update(dt);
        self.apply_scene_commands(context);
        result.map(|()| requests)
    }

    pub fn complete_ui_load(
        &mut self,
        ui: UserInterface,
        context: &mut PluginContext,
        font: Option<&FontResource>,
    ) {
        if let Some(command_host) = self.command_host.as_mut() {
            command_host.complete_ui_load(ui, context, font);
        }
    }

    pub fn fail_ui_load(&mut self) {
        if let Some(command_host) = self.command_host.as_mut() {
            command_host.fail_ui_load();
        }
    }
    pub fn runtime(&self) -> Option<&LuaRuntime> {
        self.runtime.as_ref()
    }
    pub fn runtime_mut(&mut self) -> Option<&mut LuaRuntime> {
        self.runtime.as_mut()
    }

    /// Drains Lua diagnostics collected by the runtime and host lifecycle.
    pub fn take_errors(&mut self) -> Vec<LuaErrorReport> {
        self.collect_runtime_errors();
        self.errors.drain()
    }

    fn collect_runtime_errors(&mut self) {
        let reports = self
            .runtime
            .as_mut()
            .map(LuaRuntime::take_errors)
            .unwrap_or_default();
        self.append_errors(reports);
    }

    fn append_errors(&mut self, reports: impl IntoIterator<Item = LuaErrorReport>) {
        for report in reports {
            self.errors.push(report);
        }
    }

    fn record_host_error(&mut self, phase: &str, error: impl std::fmt::Display) {
        let message = bounded_message(error);
        fyrox::core::log::Log::warn(format!(
            "[Lua] recovered host error: phase={}, error={}",
            phase, message
        ));
        let frame = self
            .runtime
            .as_ref()
            .map(LuaRuntime::update_ticks)
            .unwrap_or(0);
        self.errors.push(LuaErrorReport {
            phase: phase.to_owned(),
            script: None,
            instance_id: None,
            scope: None,
            frame,
            message,
            recovered: true,
        });
    }

    /// Registers a game-owned API.  Registration is applied immediately when
    /// the host is already running and is also retained for the next runtime.
    pub fn register_api<A: LuaApi>(&mut self, api: A) -> mlua::Result<()> {
        if let Some(runtime) = self.runtime.as_ref() {
            api.register(&runtime.lua)?;
        }
        self.api_registry.add(api);
        Ok(())
    }

    /// Semantic alias for game-owned registrations.
    pub fn register_game_api<A: LuaApi>(&mut self, api: A) -> mlua::Result<()> {
        self.register_api(api)
    }

    /// Convenience form for registering a closure from a game crate.
    pub fn register_api_fn<F>(&mut self, register: F) -> mlua::Result<()>
    where
        F: Fn(&mlua::Lua) -> mlua::Result<()> + 'static,
    {
        self.register_api(crate::LuaApiFn::new(register))
    }

    /// Semantic alias for closure-based game registrations.
    pub fn register_game_api_fn<F>(&mut self, register: F) -> mlua::Result<()>
    where
        F: Fn(&mlua::Lua) -> mlua::Result<()> + 'static,
    {
        self.register_api_fn(register)
    }

    /// Registers an engine-owned extension selected by the business binding
    /// manifest. The implementation still lives in `lua-plugin`; this name
    /// keeps generated business code from conflating it with game APIs.
    pub fn register_custom_binding_fn<F>(&mut self, register: F) -> mlua::Result<()>
    where
        F: Fn(&mlua::Lua) -> mlua::Result<()> + 'static,
    {
        self.register_api_fn(register)
    }

    pub fn api_registry(&self) -> &LuaApiRegistry {
        &self.api_registry
    }

    fn compose_api<A: LuaApi>(&self, api: A) -> LuaApiRegistry {
        let mut registry = LuaApiRegistry::new();
        registry.add(api);
        registry.extend(&self.api_registry);
        registry
    }

    pub fn start_root(&mut self) -> mlua::Result<usize> {
        self.start_root_with_api(Api {
            bridge: self.bridge.0.clone(),
        })
    }

    pub fn start_root_with_api<A: crate::LuaGameApi>(&mut self, api: A) -> mlua::Result<usize> {
        let runtime_result = match self.source_loader.clone() {
            Some(loader) => LuaRuntime::new_with_source_loader(
                self.config.clone(),
                self.compose_api(api),
                loader,
            ),
            None => LuaRuntime::new(self.config.clone(), self.compose_api(api)),
        };
        let mut runtime = match runtime_result {
            Ok(runtime) => runtime,
            Err(error) => {
                self.record_host_error("start_root", error);
                return Ok(0);
            }
        };
        if let Err(error) = runtime.start() {
            self.record_host_error("start_root", error);
            let _ = runtime.destroy();
            self.append_errors(runtime.take_errors());
            return Ok(0);
        }
        if !runtime.is_running() {
            self.record_host_error("start_root", "Lua error policy stopped the runtime");
            let _ = runtime.destroy();
            self.append_errors(runtime.take_errors());
            return Ok(0);
        }
        if !runtime.has_main_script() {
            let _ = runtime.destroy();
            return Ok(0);
        }
        let count = runtime.script_count();
        self.runtime = Some(runtime);
        Ok(count)
    }

    pub fn start_scene(
        &mut self,
        scene: &fyrox::scene::Scene,
        id_prefix: &Path,
    ) -> mlua::Result<usize> {
        self.start_scene_with_api(
            scene,
            id_prefix,
            Api {
                bridge: self.bridge.0.clone(),
            },
        )
    }

    pub fn start_scene_with_api<A: crate::LuaGameApi>(
        &mut self,
        scene: &fyrox::scene::Scene,
        id_prefix: &Path,
        api: A,
    ) -> mlua::Result<usize> {
        let runtime_result = match self.source_loader.clone() {
            Some(loader) => LuaRuntime::new_for_scene_with_source_loader(
                self.config.clone(),
                self.compose_api(api),
                loader,
            ),
            None => LuaRuntime::new_for_scene(self.config.clone(), self.compose_api(api)),
        };
        let mut runtime = match runtime_result {
            Ok(runtime) => runtime,
            Err(error) => {
                self.record_host_error("start_scene", error);
                return Ok(0);
            }
        };
        let count = match runtime.load_scene_components(scene, id_prefix) {
            Ok(count) => count,
            Err(error) => {
                self.record_host_error("load_scene_components", error);
                let _ = runtime.destroy();
                self.append_errors(runtime.take_errors());
                return Ok(0);
            }
        };
        if let Err(error) = runtime.start() {
            self.record_host_error("start_scene", error);
            let _ = runtime.destroy();
            self.append_errors(runtime.take_errors());
            return Ok(0);
        }
        if !runtime.is_running() {
            self.record_host_error("start_scene", "Lua error policy stopped the runtime");
            let _ = runtime.destroy();
            self.append_errors(runtime.take_errors());
            return Ok(0);
        }
        if !runtime.has_main_script() {
            let _ = runtime.destroy();
            return Ok(0);
        }
        self.runtime = Some(runtime);
        Ok(count)
    }

    pub fn update(&mut self, dt: f32) -> mlua::Result<()> {
        if let Some(runtime) = self.runtime.as_mut() {
            if let Err(error) = runtime.call_all("update", dt) {
                self.record_host_error("update", error);
            }
        }
        self.collect_runtime_errors();
        Ok(())
    }

    pub fn dispatch_event(&mut self, name: &str, payload: &str) -> mlua::Result<()> {
        if let Some(runtime) = self.runtime.as_mut() {
            if let Err(error) = runtime.dispatch_script_event(name, payload) {
                self.record_host_error("event", error);
            }
        }
        self.collect_runtime_errors();
        Ok(())
    }

    pub fn dispatch_ui_click(&mut self, id: &str) -> mlua::Result<bool> {
        let result = match self.runtime.as_mut() {
            Some(runtime) => runtime.dispatch_ui_click(id),
            None => Ok(false),
        };
        let result = match result {
            Ok(value) => value,
            Err(error) => {
                self.record_host_error("ui_click", error);
                false
            }
        };
        self.collect_runtime_errors();
        Ok(result)
    }

    pub fn destroy(&mut self) -> mlua::Result<()> {
        let result = self.runtime.take().map_or(Ok(()), |mut runtime| {
            let result = runtime.destroy();
            let reports = runtime.take_errors();
            self.append_errors(reports);
            result
        });
        // Clear both queues through one borrow.  Apart from being cheaper this
        // keeps teardown recoverable if a callback temporarily holds the
        // bridge borrow.
        let queues_cleared = {
            let result = self.bridge.0.try_borrow_mut();
            if let Ok(mut bridge) = result {
                bridge.clear_commands();
                true
            } else {
                false
            }
        };
        if !queues_cleared {
            self.record_host_error("destroy", "bridge queue is still borrowed; queues retained");
        }
        self.command_host = None;
        if let Err(error) = result {
            self.record_host_error("destroy", error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::LuaPluginHost;
    use crate::LuaConfig;

    #[test]
    fn host_owns_runtime_lifecycle() {
        let root = std::env::temp_dir().join(format!("lua-plugin-host-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("main.lua"),
            r#"local main = {}; function main.start() _G.host_main_started = true end; return main"#,
        )
        .unwrap();
        std::fs::write(
            root.join("host_test.lua"),
            r#"local C = {}; C.__index = C
function C.new(class) return setmetatable({}, class) end
function C:start() _G.host_started = true end
function C:update(dt) _G.host_dt = dt end
function C:on_event(name, payload) _G.host_event = name .. payload end
function C:on_destroy() _G.host_destroyed = true end
return C"#,
        )
        .unwrap();

        let mut config = LuaConfig::default();
        config.script_root = root.clone();
        let mut host = LuaPluginHost::new(config);
        host.register_api_fn(|lua| lua.globals().set("host_api", "registered"))
            .unwrap();
        host.register_custom_binding_fn(|lua| lua.globals().set("engine_extension", true))
            .unwrap();
        assert_eq!(host.start_root().unwrap(), 0);
        host.update(0.25).unwrap();
        assert!(!host
            .runtime()
            .unwrap()
            .lua
            .globals()
            .get::<bool>("host_started")
            .unwrap_or(false));
        assert!(host
            .runtime()
            .unwrap()
            .lua
            .globals()
            .get::<bool>("host_main_started")
            .unwrap());
        assert_eq!(
            host.runtime()
                .unwrap()
                .lua
                .globals()
                .get::<String>("host_api")
                .unwrap(),
            "registered"
        );
        assert!(host
            .runtime()
            .unwrap()
            .lua
            .globals()
            .get::<bool>("engine_extension")
            .unwrap());
        host.register_api_fn(|lua| lua.globals().set("late_api", true))
            .unwrap();
        assert!(host
            .runtime()
            .unwrap()
            .lua
            .globals()
            .get::<bool>("late_api")
            .unwrap());
        host.destroy().unwrap();
        assert!(host.runtime().is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn host_does_not_initialize_without_main_script() {
        let root =
            std::env::temp_dir().join(format!("lua-plugin-host-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("script.lua"),
            "return { new = function(class) return class end }",
        )
        .unwrap();
        let mut config = LuaConfig::default();
        config.script_root = root.clone();
        let mut host = LuaPluginHost::new(config);
        assert_eq!(host.start_root().unwrap(), 0);
        assert!(host.runtime().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lua_callback_error_is_recovered_and_reported() {
        let root =
            std::env::temp_dir().join(format!("lua-plugin-error-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("main.lua"),
            r#"local main = {}
function main.update() error("intentional callback failure") end
return main"#,
        )
        .unwrap();
        let mut config = LuaConfig::default();
        config.script_root = root.clone();
        let mut host = LuaPluginHost::new(config);
        assert_eq!(host.start_root().unwrap(), 0);
        host.update(1.0 / 60.0).unwrap();
        let errors = host.take_errors();
        assert!(errors.iter().any(|error| {
            error.phase == "main.update" && error.recovered && error.message.contains("intentional")
        }));
        // A second frame is still a valid host operation after the failure.
        host.update(1.0 / 60.0).unwrap();
        host.destroy().unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}
