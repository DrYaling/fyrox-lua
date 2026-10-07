//! 模块化 Lua 接入层。引擎 API 通过手写注册入口显式导出，不分析 Lua 源码。
mod api;
mod batch_api;
#[cfg(feature = "benchmark")]
mod benchmark;
mod bindings;
mod component;
mod config;
pub mod custom_bindings;
mod diagnostics;
mod embedded;
mod game_api;
mod handles;
mod host;
mod lua_config;
mod plugin;
mod resource;
mod runtime;
mod scene;
mod ui;
mod value_bindings;

pub use api::{EngineApi, LuaApi, LuaApiFn, LuaApiRegistry, LuaGameApi};
#[cfg(feature = "benchmark")]
pub use benchmark::{
    run_rust_baseline, run_rust_ui_layout, run_rust_ui_layout_legacy, BenchmarkMetric,
    RustBenchmarkReport,
};
pub use bindings::register_engine_bindings;
pub use component::{ComponentBinding, ComponentBindings, ComponentType};
pub use config::{BindingMode, LuaConfig, LuaErrorPolicy};
pub use diagnostics::LuaErrorReport;
pub use game_api::{
    Api, Bridge, BridgeHandle, BridgeRef, BridgeStats, LuaLogLevel, SceneCommand, ScopedNodeName,
    SharedName, UiCommand, UiElementKind, UiElementSpec,
};
pub use handles::HandleToken;
pub use host::LuaCommandHost;
pub use lua_config::initialize_editor_lua;
pub use plugin::LuaPluginHost;
pub use resource::{
    register_fyrox_resources, register_lua_resource_loader, EditorScript, LuaComponent, LuaScript,
    LuaScriptLoader, LuaScriptParameter,
};
pub use runtime::{LuaRuntime, LuaSourceLoader};
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
        assert_eq!(runtime.loaded_script_count(), 1);
        assert_eq!(runtime.lua.globals().get::<u32>("main_awake").unwrap(), 1);
        runtime.start().unwrap();
        runtime.call_all("update", 0.25).unwrap();
        assert_eq!(runtime.lua.globals().get::<u32>("main_start").unwrap(), 1);
        assert_eq!(runtime.lua.globals().get::<f32>("main_dt").unwrap(), 0.25);
        runtime.destroy().unwrap();
        assert_eq!(runtime.loaded_script_count(), 0);
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
        assert_eq!(runtime.loaded_script_count(), 1);
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
    fn host_source_loader_supplies_modules_without_filesystem_access() {
        use std::{collections::HashMap, path::Path, rc::Rc};
        let sources = Rc::new(HashMap::from([
            (
                "data/scripts/main.lua".to_owned(),
                "local M={}; function M:on_awake() _G.loader_main=true end; return M".to_owned(),
            ),
            (
                "data/scripts/modules/value.lua".to_owned(),
                "return { value = 73 }".to_owned(),
            ),
        ]));
        let loader_sources = sources.clone();
        let loader: LuaSourceLoader = Rc::new(move |path: &Path| {
            loader_sources
                .get(&path.to_string_lossy().replace('\\', "/"))
                .cloned()
                .ok_or_else(|| mlua::Error::runtime(format!("missing source: {}", path.display())))
        });
        let mut runtime =
            LuaRuntime::new_with_source_loader(LuaConfig::default(), Api, loader).unwrap();
        runtime
            .load_script_source(
                Path::new("consumer.lua"),
                "local C={}; function C.new(class) local m=import('modules.value'); _G.loader_value=m.value; return class end; return C",
                &Default::default(),
            )
            .unwrap();
        assert!(runtime.lua.globals().get::<bool>("loader_main").unwrap());
        runtime.start().unwrap();
        assert_eq!(
            runtime.lua.globals().get::<i64>("loader_value").unwrap(),
            73
        );
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
    fn error_policy_config_is_backward_compatible_and_explicit() {
        let c: LuaConfig = toml::from_str(
            "script_root='data/scripts'\nenabled=true\nerror_policy='log_and_continue'\nmax_errors_per_script=5",
        )
        .unwrap();
        assert_eq!(c.error_policy, LuaErrorPolicy::LogAndContinue);
        assert_eq!(c.max_errors_per_script, 5);
        let defaults: LuaConfig = toml::from_str("script_root='data/scripts'").unwrap();
        assert_eq!(defaults.error_policy, LuaErrorPolicy::DisableScript);
        assert_eq!(defaults.max_errors_per_script, 3);
        assert_eq!(defaults.max_script_bytes, 4 * 1024 * 1024);
    }

    #[test]
    fn oversized_script_source_is_rejected_before_lua_parse() {
        let mut config = LuaConfig::default();
        config.max_script_bytes = 8;
        let mut runtime = LuaRuntime::new(config, Api).unwrap();
        let error = runtime
            .load_script_source(
                std::path::Path::new("oversized.lua"),
                "return { this_source_is_too_large = true }",
                &EditorScript::default(),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("max_script_bytes"));
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
    fn bridge_rejects_non_finite_engine_values_atomically() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let lua = Lua::new();
        crate::Api {
            bridge: bridge.clone(),
        }
        .register(&lua)
        .unwrap();
        let error = lua
            .load(r#"ui.batch_compact({ { 7, "title", 0/0, 2, 100, 40 } })"#)
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("non-finite"));
        assert!(bridge.borrow().commands.is_empty());

        let error = lua
            .load(r#"scene.batch_compact({ { 5, "node", 0/0, 2, 3, 0, 0, 0, 1, 1, 1 } })"#)
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("non-finite"));
        assert!(bridge.borrow().scene_commands.is_empty());
    }

    #[test]
    fn ui_properties_are_readable_immediately_after_assignment() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let runtime = LuaRuntime::new_for_scene(
            LuaConfig::default(),
            crate::Api {
                bridge: bridge.clone(),
            },
        )
        .unwrap();
        runtime
            .lua
            .load(
                r#"
                local txt = ui.text("immediate")
                txt.text = "qwe"
                local t = txt.text
                assert(t == "qwe")
                txt:set_text("xyz")
                assert(txt.text == "xyz")
                txt.position = Vector2.new(10, 20)
                assert(txt.position.x == 10 and txt.position.y == 20)
                txt.color = Color.new(0.1, 0.2, 0.3, 1.0)
                assert(math.abs(txt.color.g - 0.2) < 0.0001)
                txt.opacity = 0.5
                assert(txt.opacity == 0.5)
                txt.visible = false
                assert(txt.visible == false)
                txt.enabled = true
                assert(txt.enabled == true)
                txt.width = 100
                assert(txt.width == 100)
                txt.height = 40
                assert(txt.height == 40)
                ui.batch_ops("immediate", {
                    { 1, "batched" },
                    { 7, 1, 2, 120, 48 },
                    { 8, 0.4, 0.5, 0.6, 1.0, 0.75 },
                })
                assert(txt.text == "batched")
                assert(txt.position.x == 1 and txt.position.y == 2)
                assert(txt.width == 120 and txt.height == 48)
                assert(math.abs(txt.color.g - 0.5) < 0.0001)
                assert(txt.opacity == 0.75)
                "#,
            )
            .exec()
            .unwrap();
        let bridge = bridge.borrow();
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetText(id, value) if id.as_ref() == "immediate" && value == "xyz")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetPosition(id, 10.0, 20.0) if id.as_ref() == "immediate")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetColor(id, r, g, b, a) if id.as_ref() == "immediate" && *r == 0.1 && *g == 0.2 && *b == 0.3 && *a == 1.0)
        ));
    }

    #[test]
    fn composite_commands_count_toward_semantic_work_budget() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let lua = Lua::new();
        crate::Api {
            bridge: bridge.clone(),
        }
        .register(&lua)
        .unwrap();
        let error = lua
            .load(
                r#"
                local operations = {}
                for i = 1, 6667 do operations[i] = {"set_layout", 1, 2, 3, 4} end
                ui.batch_ops("node", operations)
                "#,
            )
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("command buffer limit exceeded"));
        assert!(bridge.borrow().commands.is_empty());

        let error = lua
            .load(
                r#"
                local node = scene.find("Root")
                local position, rotation, scale = Vector3.new(1, 2, 3), Vector3.new(0, 0, 0), Vector3.new(1, 1, 1)
                for i = 1, 6667 do node:set_transform(position, rotation, scale) end
                "#,
            )
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("command buffer limit exceeded"));
        assert_eq!(bridge.borrow().scene_commands.len(), 6667);
    }

    #[test]
    fn batch_parsing_is_atomic_and_rejects_metamethod_protocol_fields() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let lua = Lua::new();
        crate::Api {
            bridge: bridge.clone(),
        }
        .register(&lua)
        .unwrap();
        let error = lua
            .load(
                r#"
                local mt = { __index = function() error("metamethod must not run") end }
                local ops = setmetatable({
                    { op = "set_text", id = "ok", value = "x" },
                    setmetatable({ op = "bad", id = "bad" }, mt)
                }, mt)
                ui.batch(ops)
                "#,
            )
            .exec()
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported ui.batch op") || error.contains("missing"));
        assert!(bridge.borrow().commands.is_empty());

        for source in [
            r#"ui.batch_compact({ {"set_text", "ok", "x"}, {"bad", "id", "x"} })"#,
            r#"ui.batch_ops("ok", { {"set_text", "x"}, {"bad", "x"} })"#,
        ] {
            let error = lua.load(source).exec().unwrap_err().to_string();
            assert!(error.contains("unsupported ui.batch op"));
            assert!(bridge.borrow().commands.is_empty());
        }
        lua.load(
            r#"
            ui.batch_compact({ {1, "coded", "text"}, {2, "coded", true}, {8, "coded", 1, 0, 0, 1, 0.5} })
            scene.batch_ops("coded_node", { {1, 1, 2, 3}, {5, 4, 5, 6, 0, 0, 0.5, 1, 1, 1} })
            "#,
        )
        .exec()
        .unwrap();
        assert!(bridge.borrow().commands.iter().any(
            |command| matches!(command, UiCommand::SetText(id, value) if id.as_ref() == "coded" && value == "text")
        ));
        assert!(bridge.borrow().scene_commands.iter().any(
            |command| matches!(command, SceneCommand::SetTransform(target, _, _, _) if target.name.as_ref() == "coded_node")
        ));
    }

    #[test]
    fn batch_reuses_repeated_target_name_allocation() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let lua = Lua::new();
        crate::Api {
            bridge: bridge.clone(),
        }
        .register(&lua)
        .unwrap();
        lua.load(
            r#"
            ui.batch_compact({
                {1, "same-target", "first"},
                {2, "same-target", true},
                {7, "same-target", 1, 2, 100, 40},
            })
            "#,
        )
        .exec()
        .unwrap();
        let bridge = bridge.borrow();
        let mut names = bridge.commands.iter().filter_map(|command| match command {
            UiCommand::SetText(id, _) | UiCommand::SetVisible(id, _) => Some(id),
            UiCommand::SetLayout(id, ..) => Some(id),
            _ => None,
        });
        let first = names.next().expect("first target");
        assert!(names.all(|name| std::rc::Rc::ptr_eq(first, name)));
    }

    #[test]
    fn clearing_bridge_drops_immediate_ui_state() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let lua = Lua::new();
        crate::Api {
            bridge: bridge.clone(),
        }
        .register(&lua)
        .unwrap();
        lua.load(
            r#"
            local txt = ui.component("temporary")
            txt.text = "must not survive destroy"
            txt.position = Vector2.new(3, 4)
            assert(txt.text == "must not survive destroy")
            "#,
        )
        .exec()
        .unwrap();
        assert!(bridge.borrow().ui_state("temporary").is_some());
        bridge.borrow_mut().clear_commands();
        let bridge = bridge.borrow();
        assert!(bridge.commands.is_empty());
        assert!(bridge.ui_state("temporary").is_none());
        assert!(bridge.ui_text.is_empty());
    }

    #[test]
    fn committed_ui_text_cache_is_bounded_for_dynamic_names() {
        use crate::game_api::MAX_UI_TEXT_CACHE;

        let mut bridge = Bridge::default();
        for index in 0..(MAX_UI_TEXT_CACHE + 32) {
            let id: std::rc::Rc<str> = format!("dynamic-{index}").into();
            bridge.commit_ui_text_commands(&[UiCommand::SetText(id, String::from("value"))]);
        }
        assert_eq!(bridge.ui_text.len(), MAX_UI_TEXT_CACHE);
        assert_eq!(bridge.ui_text_pending.len(), MAX_UI_TEXT_CACHE);
        assert!(!bridge.ui_text.contains_key("dynamic-0"));
        assert!(bridge.ui_text.contains_key("dynamic-4127"));
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
    fn ui_click_callback_can_be_removed_without_waiting_for_gc() {
        let mut runtime = LuaRuntime::new_for_scene(
            LuaConfig::default(),
            crate::Api {
                bridge: std::rc::Rc::new(std::cell::RefCell::new(Bridge::default())),
            },
        )
        .unwrap();
        runtime
            .lua
            .load(
                r#"
                local button = ui.find("temporary_button")
                button:on_click(function() _G.should_not_run = true end)
                button:off_click()
                "#,
            )
            .exec()
            .unwrap();
        runtime.start().unwrap();
        assert!(!runtime.dispatch_ui_click("temporary_button").unwrap());
        assert!(!runtime
            .lua
            .globals()
            .get::<bool>("should_not_run")
            .unwrap_or(false));
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
        let runtime = LuaRuntime::new_for_scene(
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
        let fast: mlua::Table = runtime.lua.globals().get("value_fast").unwrap();
        let fast_vector: mlua::Table = fast
            .get::<mlua::Function>("vector3")
            .unwrap()
            .call((1.0_f32, 2.0_f32, 3.0_f32))
            .unwrap();
        assert_eq!(fast_vector.raw_get::<f32>("z").unwrap(), 3.0);
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
                box:set_transform(Vector3.new(4, 5, 6), Vector3.new(0.4, 0.5, 0.6), Vector3.new(3, 3, 3))
                box:set_enabled(true)
                "#,
            )
            .exec()
            .unwrap();

        runtime
            .lua
            .load(
                r#"
                ui.batch({
                    {op = "set_text", id = "title", value = "batched"},
                    {op = "set_visible", id = "title", value = true},
                    {op = "set_enabled", id = "title", value = false},
                })
                scene.batch({
                    {op = "set_position", name = "BoxB", x = 1, y = 2, z = 3},
                    {op = "set_rotation", name = "BoxB", roll = 0, pitch = 0, yaw = 1},
                })
                ui.batch_compact({
                    {"set_layout", "compact", 1, 2, 30, 40},
                    {"set_tint", "compact", 0.1, 0.2, 0.3, 1, 0.8},
                })
                ui.batch_ops("grouped", {
                    {"set_text", "grouped text"},
                    {"set_enabled", true},
                })
                scene.batch_compact({
                    {"set_position", "CompactNode", 1, 2, 3},
                    {"set_transform", "CompactNode", 4, 5, 6, 0.4, 0.5, 0.6, 3, 3, 3},
                })
                scene.batch_ops("GroupedNode", {
                    {"set_enabled", false},
                })
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
            .any(|command| matches!(command, UiCommand::SetText(id, value) if id.as_ref() == "title" && value == "hello")));
        assert!(bridge
            .commands
            .iter()
            .any(|command| matches!(command, UiCommand::SetText(id, value) if id.as_ref() == "title" && value == "batched")));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetEnabled(id, true) if id.as_ref() == "title")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetLayout(id, 1.0, 2.0, 30.0, 40.0) if id.as_ref() == "compact")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetTint(id, 0.1, 0.2, 0.3, 1.0, 0.8) if id.as_ref() == "compact")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetText(id, value) if id.as_ref() == "grouped" && value == "grouped text")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetPosition(id, 10.0, 20.0) if id.as_ref() == "title")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetChecked(id, true) if id.as_ref() == "toggle")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetSelected(id, Some(2)) if id.as_ref() == "selector")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetScroll(id, 3.0, 4.0) if id.as_ref() == "scroll")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetProgress(id, 0.75) if id.as_ref() == "progress")
        ));
        assert!(bridge.commands.iter().any(
            |command| matches!(command, UiCommand::SetPopupOpen(id, true) if id.as_ref() == "popup")
        ));
        assert!(bridge
            .scene_commands
            .iter()
            .any(|command| matches!(command, SceneCommand::SetRotationAngles(target, 0.1, 0.2, 0.3) if target.name.as_ref() == "BoxB")));
        assert!(bridge.scene_commands.iter().any(
            |command| matches!(command, SceneCommand::SetScale(target, 2.0, 2.0, 2.0) if target.name.as_ref() == "BoxB")
        ));
        assert!(bridge.scene_commands.iter().any(
            |command| matches!(command, SceneCommand::SetTransform(target, position, rotation, scale)
                if target.name.as_ref() == "BoxB"
                    && position.x == 4.0 && position.y == 5.0 && position.z == 6.0
                    && rotation.x == 0.4 && rotation.y == 0.5 && rotation.z == 0.6
                    && scale.x == 3.0 && scale.y == 3.0 && scale.z == 3.0)
        ));
        assert!(bridge.scene_commands.iter().any(
            |command| matches!(command, SceneCommand::SetPosition(target, 1.0, 2.0, 3.0) if target.name.as_ref() == "CompactNode")
        ));
        assert!(bridge.scene_commands.iter().any(
            |command| matches!(command, SceneCommand::SetTransform(target, position, rotation, scale)
                if target.name.as_ref() == "CompactNode"
                    && position.x == 4.0 && rotation.z == 0.6 && scale.x == 3.0)
        ));
        assert!(bridge.scene_commands.iter().any(
            |command| matches!(command, SceneCommand::SetEnabled(target, false) if target.name.as_ref() == "GroupedNode")
        ));
    }

    #[test]
    fn translator_caches_use_weak_values_and_collect_unreferenced_proxies() {
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(Bridge::default()));
        let runtime = LuaRuntime::new_for_scene(
            LuaConfig::default(),
            crate::Api {
                bridge: bridge.clone(),
            },
        )
        .unwrap();
        runtime
            .lua
            .load(
                r#"
                for i = 1, 100 do
                    local value = ui.find("ephemeral_" .. i)
                end
                collectgarbage("collect")
                "#,
            )
            .exec()
            .unwrap();
        let cache: mlua::Table = runtime.lua.globals().get("__fwok_ui_components").unwrap();
        assert!(cache
            .pairs::<mlua::Value, mlua::AnyUserData>()
            .next()
            .is_none());
        assert_eq!(bridge.borrow().commands.len(), 100);
    }
}
