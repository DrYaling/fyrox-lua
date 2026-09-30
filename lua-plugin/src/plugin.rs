//! Main-thread Lua plugin host.

use crate::{Api, BridgeHandle, LuaConfig, LuaRuntime};
use std::{path::Path, rc::Rc, thread::ThreadId};

pub struct LuaPluginHost {
    config: LuaConfig,
    bridge: BridgeHandle,
    runtime: Option<LuaRuntime>,
    owner_thread: ThreadId,
    _main_thread_only: Rc<()>,
}

impl std::fmt::Debug for LuaPluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaPluginHost")
            .field("runtime_active", &self.runtime.is_some())
            .field("owner_thread", &self.owner_thread)
            .finish()
    }
}

impl LuaPluginHost {
    pub fn new(config: LuaConfig) -> Self {
        Self {
            config,
            bridge: BridgeHandle::default(),
            runtime: None,
            owner_thread: std::thread::current().id(),
            _main_thread_only: Rc::new(()),
        }
    }

    fn assert_owner_thread(&self) {
        assert_eq!(
            self.owner_thread,
            std::thread::current().id(),
            "LuaPluginHost used from a non-owner thread"
        );
    }

    pub fn config(&self) -> &LuaConfig {
        self.assert_owner_thread();
        &self.config
    }
    pub fn bridge(&self) -> &BridgeHandle {
        self.assert_owner_thread();
        &self.bridge
    }
    pub fn runtime(&self) -> Option<&LuaRuntime> {
        self.assert_owner_thread();
        self.runtime.as_ref()
    }
    pub fn runtime_mut(&mut self) -> Option<&mut LuaRuntime> {
        self.assert_owner_thread();
        self.runtime.as_mut()
    }

    pub fn start_root(&mut self) -> mlua::Result<usize> {
        self.assert_owner_thread();
        if !self.config.script_root.join("main.lua").is_file() {
            return Ok(0);
        }
        let mut runtime = LuaRuntime::new(
            self.config.clone(),
            Api {
                bridge: self.bridge.0.clone(),
            },
        )?;
        runtime.start()?;
        let count = runtime.script_count();
        self.runtime = Some(runtime);
        Ok(count)
    }

    pub fn start_scene(
        &mut self,
        scene: &fyrox::scene::Scene,
        id_prefix: &Path,
    ) -> mlua::Result<usize> {
        self.assert_owner_thread();
        if !self.config.script_root.join("main.lua").is_file() {
            return Ok(0);
        }
        let mut runtime = LuaRuntime::new_for_scene(
            self.config.clone(),
            Api {
                bridge: self.bridge.0.clone(),
            },
        )?;
        runtime.load_root(&self.config.script_root)?;
        let count = runtime.load_scene_components(scene, id_prefix)?;
        runtime.start()?;
        self.runtime = Some(runtime);
        Ok(count)
    }

    pub fn update(&mut self, dt: f32) -> mlua::Result<()> {
        self.assert_owner_thread();
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.call_all("update", dt)?;
        }
        Ok(())
    }

    pub fn dispatch_event(&mut self, name: &str, payload: &str) -> mlua::Result<()> {
        self.assert_owner_thread();
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.dispatch_script_event(name, payload)?;
        }
        Ok(())
    }

    pub fn dispatch_ui_click(&mut self, id: &str) -> mlua::Result<bool> {
        self.assert_owner_thread();
        match self.runtime.as_mut() {
            Some(runtime) => runtime.dispatch_ui_click(id),
            None => Ok(false),
        }
    }

    pub fn destroy(&mut self) -> mlua::Result<()> {
        self.assert_owner_thread();
        let result = self
            .runtime
            .take()
            .map_or(Ok(()), |mut runtime| runtime.destroy());
        self.bridge.0.borrow_mut().commands.clear();
        self.bridge.0.borrow_mut().scene_commands.clear();
        result
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
        assert_eq!(host.start_root().unwrap(), 1);
        host.update(0.25).unwrap();
        host.dispatch_event("host.", "ok").unwrap();
        assert!(host
            .runtime()
            .unwrap()
            .lua
            .globals()
            .get::<bool>("host_started")
            .unwrap());
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
                .get::<f32>("host_dt")
                .unwrap(),
            0.25
        );
        assert_eq!(
            host.runtime()
                .unwrap()
                .lua
                .globals()
                .get::<String>("host_event")
                .unwrap(),
            "host.ok"
        );
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
}
