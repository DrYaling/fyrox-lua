//! 模块化 Lua 接入层。引擎 API 通过手写注册入口显式导出，不分析 Lua 源码。
mod api;
#[cfg(feature = "benchmark")]
mod benchmark;
mod bindings;
mod component;
mod config;
mod game_api;
mod handles;
mod plugin;
mod resource;
mod runtime;
mod scene;
mod ui;
mod value_bindings;

pub use api::LuaGameApi;
#[cfg(feature = "benchmark")]
pub use benchmark::{run_rust_baseline, BenchmarkMetric, RustBenchmarkReport};
pub use bindings::register_engine_bindings;
pub use component::{ComponentBinding, ComponentBindings, ComponentType};
pub use config::{BindingMode, LuaConfig};
pub use game_api::{
    Api, Bridge, BridgeHandle, BridgeRef, LuaLogLevel, SceneCommand, ScopedNodeName, UiCommand,
    UiElementKind, UiElementSpec,
};
pub use handles::HandleToken;
pub use plugin::LuaPluginHost;
pub use resource::{
    register_fyrox_resources, register_lua_resource_loader, EditorScript, LuaComponent, LuaScript,
    LuaScriptLoader, LuaScriptParameter,
};
pub use runtime::LuaRuntime;
pub use scene::SceneRegistry;
pub use ui::UiRegistry;

/// 编译期分析宏生成的绑定条目。
#[cfg(test)]
mod tests {
    use super::*;
    use fyrox::core::visitor::Visit;
    use mlua::{Lua, ObjectLike};

    struct Api;
    impl LuaGameApi for Api {
        fn register(&self, _: &Lua) -> mlua::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn class_lifecycle_and_events_work() {
        let dir = std::env::temp_dir().join(format!("fwok-lua-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.lua");
        std::fs::write(
            &file,
            r#"local C={}; C.__index=C
function C.new(class) return setmetatable({count=0},class) end
function C:on_awake() self.count=self.count+1 end
function C:start() self.count=self.count+1 end
function C:update(dt) self.count=self.count+dt end
function C:on_event(name,payload) self.last=name..payload end
function C:on_destroy() self.destroyed=true end
return C"#,
        )
        .unwrap();
        let config = LuaConfig {
            script_root: dir.clone(),
            binding_mode: BindingMode::PackageFull,
            ..Default::default()
        };
        let mut rt = LuaRuntime::new(config, Api).unwrap();
        rt.load_script(&file).unwrap();
        rt.start().unwrap();
        rt.call_all("update", 0.5).unwrap();
        rt.dispatch_script_event("x", "y").unwrap();
        rt.destroy().unwrap();
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_dir(dir);
    }
    #[test]
    fn lifecycle_methods_are_optional() {
        let dir = std::env::temp_dir().join(format!("fwok-lua-awake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("bad.lua"),
            "local C={}; function C.new(class) return class end; return C",
        )
        .unwrap();
        let c = LuaConfig {
            script_root: dir.clone(),
            binding_mode: BindingMode::PackageFull,
            ..Default::default()
        };
        let mut runtime = LuaRuntime::new(c, Api).unwrap();
        runtime.load_script(&dir.join("bad.lua")).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn optional_main_script_uses_project_lifecycle_in_same_vm() {
        let dir = std::env::temp_dir().join(format!("fwok-lua-main-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.lua"),
            r#"local Main = {}; Main.__index = Main
function Main.new(class) return setmetatable({}, class) end
function Main:on_awake() _G.main_awake = (_G.main_awake or 0) + 1 end
function Main:start() _G.main_start = (_G.main_start or 0) + 1 end
function Main:update(dt) _G.main_dt = dt end
function Main:on_destroy() _G.main_destroy = (_G.main_destroy or 0) + 1 end
return Main"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("component.lua"),
            "local C={}; function C.new(class) return class end; return C",
        )
        .unwrap();
        let mut runtime = LuaRuntime::new(
            LuaConfig {
                script_root: dir.clone(),
                ..Default::default()
            },
            Api,
        )
        .unwrap();
        assert_eq!(runtime.script_count(), 0);
        assert_eq!(runtime.lua.globals().get::<u32>("main_awake").unwrap(), 1);
        runtime.start().unwrap();
        runtime.call_all("update", 0.25).unwrap();
        assert_eq!(runtime.lua.globals().get::<u32>("main_start").unwrap(), 1);
        assert_eq!(runtime.lua.globals().get::<f32>("main_dt").unwrap(), 0.25);
        runtime.destroy().unwrap();
        assert_eq!(runtime.lua.globals().get::<u32>("main_destroy").unwrap(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreferenced_lua_file_is_not_instantiated_or_registered() {
        let dir =
            std::env::temp_dir().join(format!("fwok-lua-unreferenced-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.lua"),
            "local M={}; function M:on_awake() _G.main_awake=true end; return M",
        )
        .unwrap();
        std::fs::write(
            dir.join("unused.lua"),
            "_G.unused_loaded = (_G.unused_loaded or 0) + 1; local C={}; function C.new(class) _G.unused_instantiated=true; return class end; return C",
        )
        .unwrap();
        let runtime = LuaRuntime::new(
            LuaConfig {
                script_root: dir.clone(),
                ..Default::default()
            },
            Api,
        )
        .unwrap();
        assert!(runtime.lua.globals().get::<bool>("main_awake").unwrap());
        assert_eq!(
            runtime
                .lua
                .globals()
                .get::<u32>("unused_loaded")
                .unwrap_or(0),
            0
        );
        assert!(!runtime
            .lua
            .globals()
            .get::<bool>("unused_instantiated")
            .unwrap_or(false));
        assert_eq!(runtime.script_count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn require_loads_module_and_lua_created_state() {
        let dir = std::env::temp_dir().join(format!("fwok-lua-require-{}", std::process::id()));
        let modules = dir.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        std::fs::write(
            modules.join("helper.lua"),
            "return { make=function(v) local s={value=v}; function s:double() return self.value*2 end; return s end }",
        )
        .unwrap();
        std::fs::write(
            dir.join("consumer.lua"),
            "local C={}; C.__index=C; local h=require('modules.helper'); function C.new(class) return setmetatable({state=h.make(21)},class) end; function C:on_awake() _G.require_result=self.state:double() end; return C",
        )
        .unwrap();
        let mut runtime = LuaRuntime::new(
            LuaConfig {
                script_root: dir.clone(),
                ..Default::default()
            },
            Api,
        )
        .unwrap();
        runtime.load_script(&dir.join("consumer.lua")).unwrap();
        runtime.start().unwrap();
        assert_eq!(
            runtime.lua.globals().get::<i64>("require_result").unwrap(),
            42
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn import_uses_configured_script_root_for_component_loading() {
        let root =
            std::env::temp_dir().join(format!("fwok-lua-import-root-{}", std::process::id()));
        std::fs::create_dir_all(root.join("modules")).unwrap();
        std::fs::write(
            root.join("main.lua"),
            "local Main={}; function Main:start() _G.main_started=true end; return Main",
        )
        .unwrap();
        std::fs::write(
            root.join("modules/helper.lua"),
            "return { value = 'configured-root' }",
        )
        .unwrap();
        let mut runtime = LuaRuntime::new(
            LuaConfig {
                script_root: root.clone(),
                ..Default::default()
            },
            Api,
        )
        .unwrap();
        let component = LuaComponent {
            source_override: "local C={}; C.__index=C; local m=import('modules.helper'); function C.new(class) return setmetatable({value=m.value},class) end; function C:on_awake() _G.import_value=self.value end; return C".into(),
            ..Default::default()
        };
        runtime
            .load_component(&component, std::path::Path::new("component"))
            .unwrap();
        runtime.start().unwrap();
        assert_eq!(
            runtime.lua.globals().get::<String>("import_value").unwrap(),
            "configured-root"
        );
        assert!(runtime.lua.globals().get::<bool>("main_started").unwrap());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn editor_feature_controls_effective_mode() {
        let c = LuaConfig::default();
        assert_eq!(c.binding_mode, BindingMode::EditorReflection);
        assert_eq!(
            c.effective_binding_mode(),
            if cfg!(feature = "editor") {
                BindingMode::EditorReflection
            } else {
                BindingMode::PackageFull
            }
        );
    }
    #[test]
    fn missing_mode_uses_editor_default() {
        let c: LuaConfig = toml::from_str("script_root='data/scripts'\nenabled=true").unwrap();
        assert_eq!(c.binding_mode, BindingMode::EditorReflection);
    }

    #[test]
    fn missing_config_uses_data_scripts_as_search_root() {
        let path =
            std::env::temp_dir().join(format!("fwok-lua-config-missing-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let config = LuaConfig::load(&path).unwrap();
        assert_eq!(config.script_root, std::path::PathBuf::from("data/scripts"));
        assert!(config.enabled);
    }

    #[test]
    fn engine_value_bindings_are_executable() {
        let lua = Lua::new();
        register_engine_bindings(&lua).unwrap();
        lua.load("local v = Vector3.new(1, 2, 3); assert(v.x == 1 and v.y == 2 and v.z == 3)")
            .exec()
            .unwrap();
    }

    #[cfg(feature = "benchmark")]
    #[test]
    fn benchmark_api_reports_rust_baselines_to_lua() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let lua = Lua::new();
        crate::Api { bridge }.register(&lua).unwrap();
        let metric: mlua::Table = lua
            .load("return benchmark.rust_baseline().numeric")
            .eval()
            .unwrap();
        assert_eq!(metric.get::<u32>("iterations").unwrap(), 1000);
        assert!(metric.get::<f64>("average_ns").unwrap().is_finite());
        assert!(metric.get::<f64>("checksum").unwrap().is_finite());
    }

    #[test]
    fn bridge_rejects_commands_over_per_frame_budget() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let lua = Lua::new();
        crate::Api {
            bridge: bridge.clone(),
        }
        .register(&lua)
        .unwrap();
        let error = lua
            .load("for i = 1, 20001 do ui.load('data/test.ui') end")
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("command buffer limit exceeded"));
        assert_eq!(bridge.borrow().commands.len(), 20_000);
    }

    #[test]
    fn component_bindings_build_runtime_index_and_are_passed_to_lua() {
        use fyrox::core::pool::ErasedHandle;
        let mut bindings = ComponentBindings::default();
        bindings.entries = vec![ComponentBinding {
            component_type: ComponentType::Node,
            handle: ErasedHandle::none(),
            key: "host".into(),
        }];
        assert_eq!(bindings.get("host").unwrap().key, "host");
        assert!(bindings.get("missing").is_none());
    }

    #[test]
    fn editor_script_parameters_are_filtered_and_serializable() {
        let editor_script = EditorScript {
            enabled: true,
            parameters: vec![
                LuaScriptParameter {
                    name: "speed".into(),
                    value: "4.5".into(),
                    exported: true,
                },
                LuaScriptParameter {
                    name: "internal".into(),
                    value: "secret".into(),
                    exported: false,
                },
                LuaScriptParameter {
                    name: String::new(),
                    value: "ignored".into(),
                    exported: true,
                },
            ],
        };
        let values = editor_script.exported_parameters().collect::<Vec<_>>();
        assert_eq!(values, vec![("speed", "4.5")]);

        let mut visitor = fyrox::core::visitor::Visitor::new();
        editor_script
            .clone()
            .visit("EditorScript", &mut visitor)
            .unwrap();
    }

    #[test]
    fn component_load_injects_parameters_and_honors_enabled() {
        let mut runtime = LuaRuntime::new(LuaConfig::default(), Api).unwrap();
        let component = LuaComponent {
            script: Some(fyrox::asset::Resource::new_embedded(LuaScript {
                source: r#"local C={}; C.__index=C
function C.new(class,p) return setmetatable({speed=tonumber(p.speed)},class) end
function C:on_awake() self.awake=true end
function C:update(dt) self.value=self.speed*dt end
return C"#
                    .into(),
            })),
            source_override: String::new(),
            enabled: true,
            editor_script: EditorScript {
                enabled: true,
                parameters: vec![LuaScriptParameter {
                    name: "speed".into(),
                    value: "3".into(),
                    exported: true,
                }],
            },
            components: ComponentBindings::default(),
        };
        runtime
            .load_component(&component, std::path::Path::new("component"))
            .unwrap();
        runtime.call_all("update", 2.0).unwrap();
        let disabled = LuaComponent {
            enabled: false,
            ..component
        };
        runtime
            .load_component(&disabled, std::path::Path::new("disabled"))
            .unwrap();
    }

    #[test]
    fn component_source_override_can_be_edited_without_resource() {
        let mut runtime = LuaRuntime::new(LuaConfig::default(), Api).unwrap();
        let component = LuaComponent {
            script: None,
            source_override: "local C={}; C.__index=C; function C.new(class,p) return setmetatable({ok=p.value},class) end; function C:on_awake() _G.override_ok=self.ok end; return C".into(),
            enabled: true,
            editor_script: EditorScript { enabled: true, parameters: vec![LuaScriptParameter { name: "value".into(), value: "edited".into(), exported: true }] },
            components: ComponentBindings::default(),
        };
        runtime
            .load_component(&component, std::path::Path::new("override"))
            .unwrap();
        assert_eq!(
            runtime.lua.globals().get::<String>("override_ok").unwrap(),
            "edited"
        );
    }

    #[test]
    fn ui_click_callbacks_are_dispatchable_from_host() {
        let mut runtime = LuaRuntime::new_for_scene(LuaConfig::default(), Api).unwrap();
        let callbacks = runtime.lua.create_table().unwrap();
        callbacks
            .set(
                "button_under_test",
                runtime
                    .lua
                    .create_function(|lua, ()| lua.globals().set("ui_clicked", true))
                    .unwrap(),
            )
            .unwrap();
        runtime
            .lua
            .globals()
            .set("__fwok_ui_clicks", callbacks)
            .unwrap();

        runtime.start().unwrap();
        assert!(runtime.dispatch_ui_click("button_under_test").unwrap());
        assert!(runtime.lua.globals().get::<bool>("ui_clicked").unwrap());
        assert!(!runtime.dispatch_ui_click("missing_button").unwrap());
    }

    #[test]
    fn scene_component_scan_uses_the_real_graph_component() {
        use fyrox::{
            core::pool::Handle,
            graph::SceneGraph,
            scene::{base::BaseBuilder, node::Node, pivot::PivotBuilder, Scene},
        };

        let mut scene = Scene::new();
        let node: Handle<Node> = scene
            .graph
            .add_node(PivotBuilder::new(BaseBuilder::new().with_name("LuaHost")).build_node());
        scene.graph[node].add_script(LuaComponent {
            script: Some(fyrox::asset::Resource::new_embedded(LuaScript {
                source: r#"local C={}; C.__index=C
function C.new(class,p) return setmetatable({speed=tonumber(p.speed)},class) end
function C:on_awake() _G.awake_speed=self.speed end
function C:update(dt) _G.updated_speed=self.speed*dt end
return C"#
                    .into(),
            })),
            source_override: String::new(),
            enabled: true,
            editor_script: EditorScript {
                enabled: true,
                parameters: vec![LuaScriptParameter {
                    name: "speed".into(),
                    value: "4".into(),
                    exported: true,
                }],
            },
            components: ComponentBindings::default(),
        });

        let mut runtime = LuaRuntime::new(LuaConfig::default(), Api).unwrap();
        assert_eq!(
            runtime
                .load_scene_components(&scene, std::path::Path::new("scene"))
                .unwrap(),
            1
        );
        assert_eq!(runtime.script_count(), 1);
        runtime.start().unwrap();
        runtime.call_all("update", 0.5).unwrap();
        assert_eq!(
            runtime.lua.globals().get::<f32>("awake_speed").unwrap(),
            4.0
        );
        assert_eq!(
            runtime.lua.globals().get::<f32>("updated_speed").unwrap(),
            2.0
        );
    }

    #[test]
    fn common_ui_and_scene_userdata_enqueue_generic_operations() {
        use std::{cell::RefCell, rc::Rc};

        let bridge = Rc::new(RefCell::new(Bridge::default()));
        let mut runtime = LuaRuntime::new_for_scene(
            LuaConfig::default(),
            crate::Api {
                bridge: bridge.clone(),
            },
        )
        .unwrap();
        let ui: mlua::Table = runtime.lua.globals().get("ui").unwrap();
        assert!(ui.get::<mlua::Function>("text").is_ok());
        let scene: mlua::Table = runtime.lua.globals().get("scene").unwrap();
        assert!(scene.get::<mlua::Function>("node").is_ok());
        let vectors: mlua::Table = runtime.lua.globals().get("Vector3").unwrap();
        let vector: mlua::AnyUserData = vectors
            .get::<mlua::Function>("new")
            .unwrap()
            .call((1.0_f32, 2.0_f32, 3.0_f32))
            .unwrap();
        assert_eq!(vector.get::<f32>("x").unwrap(), 1.0);
        runtime
            .lua
            .load(
                r#"
                ui.load("data/test.ui")
                ui.show(true)
                local title = ui.text("title")
                title:set_text("hello")
                title:set_enabled(true)
                title:set_position(10, 20)
                ui.toggle("toggle"):set_checked(true)
                ui.selector("selector"):set_selected(2)
                ui.scroll_viewer("scroll"):set_scroll(3, 4)
                ui.progress_bar("progress"):set_progress(0.75)
                ui.popup("popup"):open()
                ui.image("image"):set_opacity(0.5)
                ui.grid("grid_child"):set_row(2)
                local box = scene.node("BoxB")
                local position = Vector3.new(1, 2, 3)
                box:set_position(position)
                box:set_rotation(0.1, 0.2, 0.3)
                box:set_scale(2, 2, 2)
                box:set_enabled(true)
                "#,
            )
            .exec()
            .unwrap();

        let bridge = bridge.borrow();
        assert!(matches!(bridge.commands[0], UiCommand::Load(ref path) if path == "data/test.ui"));
        assert!(matches!(bridge.commands[1], UiCommand::Show(true)));
        assert!(bridge
            .commands
            .iter()
            .any(|command| matches!(command, UiCommand::SetText(id, value) if id == "title" && value == "hello")));
        assert!(bridge
            .commands
            .iter()
            .any(|command| matches!(command, UiCommand::SetEnabled(id, true) if id == "title")));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetPosition(id, 10.0, 20.0) if id == "title")
        ));
        assert!(bridge
            .commands
            .iter()
            .any(|command| matches!(command, UiCommand::SetChecked(id, true) if id == "toggle")));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetSelected(id, Some(2)) if id == "selector")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetScroll(id, 3.0, 4.0) if id == "scroll")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetProgress(id, 0.75) if id == "progress")
        ));
        assert!(bridge
            .commands
            .iter()
            .any(|command| matches!(command, UiCommand::SetPopupOpen(id, true) if id == "popup")));
        assert!(bridge
            .scene_commands
            .iter()
            .any(|command| matches!(command, SceneCommand::SetRotationAngles(target, 0.1, 0.2, 0.3) if target.name == "BoxB")));
        assert!(bridge.scene_commands.iter().any(
            |command| matches!(command, SceneCommand::SetScale(target, 2.0, 2.0, 2.0) if target.name == "BoxB")
        ));
    }
}
