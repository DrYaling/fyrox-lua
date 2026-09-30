use crate::handles::HandleToken;
use mlua::{FromLua, Lua, UserData, UserDataFields, UserDataMethods, Value};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::rc::Rc;
#[cfg(feature = "benchmark")]
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct LuaVector2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LuaVector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LuaVector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LuaColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

macro_rules! value_userdata {
    ($name:ident, {$($field:ident),+}) => {
        impl UserData for $name {
            fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
                $(fields.add_field_method_get(stringify!($field), |_, this| Ok(this.$field));)+
                $(fields.add_field_method_set(stringify!($field), |_, this, value: f32| {
                    if !value.is_finite() { return Err(mlua::Error::runtime("value field must be finite")); }
                    this.$field = value; Ok(())
                });)+
            }
        }
    };
}

value_userdata!(LuaVector2, { x, y });
value_userdata!(LuaVector3, { x, y, z });
value_userdata!(LuaVector4, { x, y, z, w });
value_userdata!(LuaColor, { r, g, b, a });

macro_rules! value_from_lua {
    ($name:ident, $lua_name:literal, {$($field:ident),+}) => {
        impl FromLua for $name {
            fn from_lua(value: Value, _lua: &Lua) -> mlua::Result<Self> {
                match value {
                    Value::UserData(data) => Ok(*data.borrow::<Self>()?),
                    Value::Table(table) => Ok(Self {
                        $($field: table.get(stringify!($field))?),+
                    }),
                    _ => Err(mlua::Error::FromLuaConversionError {
                        from: value.type_name(),
                        to: $lua_name.to_owned(),
                        message: Some(concat!("expected ", $lua_name, " userdata or field table").into()),
                    }),
                }
            }
        }
    };
}

value_from_lua!(LuaVector2, "Vector2", { x, y });
value_from_lua!(LuaVector3, "Vector3", { x, y, z });
value_from_lua!(LuaVector4, "Vector4", { x, y, z, w });
value_from_lua!(LuaColor, "Color", { r, g, b, a });

/// Registers the value types generated bindings use for common engine arguments.
#[inline]
pub(crate) fn register_value_types(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    for (name, constructor) in [
        (
            "Vector2",
            lua.create_function(|_, (x, y): (f32, f32)| Ok(LuaVector2 { x, y }))?,
        ),
        (
            "Vector3",
            lua.create_function(|_, (x, y, z): (f32, f32, f32)| Ok(LuaVector3 { x, y, z }))?,
        ),
        (
            "Vector4",
            lua.create_function(|_, (x, y, z, w): (f32, f32, f32, f32)| {
                Ok(LuaVector4 { x, y, z, w })
            })?,
        ),
        (
            "Color",
            lua.create_function(|_, (r, g, b, a): (f32, f32, f32, f32)| {
                Ok(LuaColor { r, g, b, a })
            })?,
        ),
    ] {
        let table = lua.create_table()?;
        table.set("new", constructor)?;
        globals.set(name, table)?;
    }
    crate::value_bindings::register(lua)
}

#[derive(Default, Debug)]
pub struct Bridge {
    pub ui_text: HashMap<String, String>,
    pub commands: Vec<UiCommand>,
    pub scene_commands: Vec<SceneCommand>,
}

const MAX_COMMANDS_PER_FRAME: usize = 20_000;

#[derive(Debug, Clone)]
pub enum SceneCommand {
    Resolve(ScopedNodeName),
    SetPosition(ScopedNodeName, f32, f32, f32),
    SetRotationZ(ScopedNodeName, f32),
    SetRotationAngles(ScopedNodeName, f32, f32, f32),
    SetScale(ScopedNodeName, f32, f32, f32),
    SetEnabled(ScopedNodeName, bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopedNodeName {
    pub scope: Option<HandleToken>,
    pub name: String,
}

#[derive(Debug, Clone, Copy)]
pub enum LuaLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug)]
pub enum UiCommand {
    Load(String),
    Show(bool),
    Resolve(String),
    Create(UiElementSpec),
    SetText(String, String),
    Append(String, String),
    SetVisible(String, bool),
    SetEnabled(String, bool),
    SetWidth(String, f32),
    SetHeight(String, f32),
    SetPosition(String, f32, f32),
    SetChecked(String, bool),
    SetSelected(String, Option<usize>),
    SetScroll(String, f32, f32),
    SetProgress(String, f32),
    SetPopupOpen(String, bool),
    SetOpacity(String, f32),
    SetGridRow(String, usize),
    SetGridColumn(String, usize),
    SetColor(String, f32, f32, f32, f32),
    Log(LuaLogLevel, String),
}

#[derive(Debug)]
pub enum UiElementKind {
    Text,
    TextBox,
    Button,
}

#[derive(Debug)]
pub struct UiElementSpec {
    pub kind: UiElementKind,
    pub id: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub type BridgeRef = Rc<RefCell<Bridge>>;

#[derive(Clone)]
pub struct BridgeHandle(pub BridgeRef);

impl Default for BridgeHandle {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(Bridge::default())))
    }
}

impl std::fmt::Debug for BridgeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BridgeHandle")
    }
}
impl PartialEq for BridgeHandle {
    fn eq(&self, o: &Self) -> bool {
        Rc::ptr_eq(&self.0, &o.0)
    }
}

fn borrow_mut(b: &BridgeRef) -> mlua::Result<RefMut<'_, Bridge>> {
    let bridge = b
        .try_borrow_mut()
        .map_err(|_| mlua::Error::runtime("Lua host context is already mutably borrowed"))?;
    if bridge.commands.len() + bridge.scene_commands.len() >= MAX_COMMANDS_PER_FRAME {
        return Err(mlua::Error::runtime(format!(
            "Lua command buffer limit exceeded ({MAX_COMMANDS_PER_FRAME})"
        )));
    }
    Ok(bridge)
}

fn borrow(b: &BridgeRef) -> mlua::Result<Ref<'_, Bridge>> {
    b.try_borrow()
        .map_err(|_| mlua::Error::runtime("Lua host context is already mutably borrowed"))
}

#[derive(Clone)]
pub(crate) struct UiComponentRef {
    pub(crate) id: String,
    bridge: BridgeRef,
}

#[inline]
pub(crate) fn queue_ui_command(this: &UiComponentRef, command: UiCommand) -> mlua::Result<()> {
    borrow_mut(&this.bridge)?.commands.push(command);
    Ok(())
}

#[derive(Clone)]
struct SceneNodeRef {
    target: ScopedNodeName,
    bridge: BridgeRef,
}

impl UserData for SceneNodeRef {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("set_position", |lua, this, values: mlua::MultiValue| {
            let LuaVector3 { x, y, z } = vector3_args(lua, values)?;
            borrow_mut(&this.bridge)?
                .scene_commands
                .push(SceneCommand::SetPosition(this.target.clone(), x, y, z));
            Ok(())
        });
        m.add_method("set_rotation_z", |_, this, angle: f32| {
            borrow_mut(&this.bridge)?
                .scene_commands
                .push(SceneCommand::SetRotationZ(this.target.clone(), angle));
            Ok(())
        });
        m.add_method(
            "set_rotation",
            |_, this, (roll, pitch, yaw): (f32, f32, f32)| {
                borrow_mut(&this.bridge)?
                    .scene_commands
                    .push(SceneCommand::SetRotationAngles(
                        this.target.clone(),
                        roll,
                        pitch,
                        yaw,
                    ));
                Ok(())
            },
        );
        m.add_method("set_scale", |lua, this, values: mlua::MultiValue| {
            let LuaVector3 { x, y, z } = vector3_args(lua, values)?;
            borrow_mut(&this.bridge)?
                .scene_commands
                .push(SceneCommand::SetScale(this.target.clone(), x, y, z));
            Ok(())
        });
        m.add_method("set_enabled", |_, this, value: bool| {
            borrow_mut(&this.bridge)?
                .scene_commands
                .push(SceneCommand::SetEnabled(this.target.clone(), value));
            Ok(())
        });
    }
}

#[inline]
fn vector3_args(lua: &Lua, values: mlua::MultiValue) -> mlua::Result<LuaVector3> {
    if values.len() == 1 {
        return LuaVector3::from_lua(values.front().cloned().unwrap_or(Value::Nil), lua);
    }
    if values.len() == 3 {
        return Ok(LuaVector3 {
            x: f32::from_lua(values[0].clone(), lua)?,
            y: f32::from_lua(values[1].clone(), lua)?,
            z: f32::from_lua(values[2].clone(), lua)?,
        });
    }
    Err(mlua::Error::runtime(
        "expected Vector3 or three numeric arguments",
    ))
}

#[inline]
fn vector2_args(lua: &Lua, values: mlua::MultiValue) -> mlua::Result<LuaVector2> {
    if values.len() == 1 {
        return LuaVector2::from_lua(values.front().cloned().unwrap_or(Value::Nil), lua);
    }
    if values.len() == 2 {
        return Ok(LuaVector2 {
            x: f32::from_lua(values[0].clone(), lua)?,
            y: f32::from_lua(values[1].clone(), lua)?,
        });
    }
    Err(mlua::Error::runtime(
        "expected Vector2 or two numeric arguments",
    ))
}
impl UserData for UiComponentRef {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("set_text", |_, this, value: String| {
            borrow_mut(&this.bridge)?
                .commands
                .push(UiCommand::SetText(this.id.clone(), value));
            Ok(())
        });
        m.add_method("append", |_, this, value: String| {
            borrow_mut(&this.bridge)?
                .commands
                .push(UiCommand::Append(this.id.clone(), value));
            Ok(())
        });
        m.add_method("set_visible", |_, this, value: bool| {
            borrow_mut(&this.bridge)?
                .commands
                .push(UiCommand::SetVisible(this.id.clone(), value));
            Ok(())
        });
        m.add_method("set_enabled", |_, this, value: bool| {
            borrow_mut(&this.bridge)?
                .commands
                .push(UiCommand::SetEnabled(this.id.clone(), value));
            Ok(())
        });
        m.add_method("set_width", |_, this, value: f32| {
            borrow_mut(&this.bridge)?
                .commands
                .push(UiCommand::SetWidth(this.id.clone(), value));
            Ok(())
        });
        m.add_method("set_height", |_, this, value: f32| {
            borrow_mut(&this.bridge)?
                .commands
                .push(UiCommand::SetHeight(this.id.clone(), value));
            Ok(())
        });
        m.add_method("set_position", |lua, this, values: mlua::MultiValue| {
            let LuaVector2 { x, y } = vector2_args(lua, values)?;
            borrow_mut(&this.bridge)?
                .commands
                .push(UiCommand::SetPosition(this.id.clone(), x, y));
            Ok(())
        });
        m.add_method("set_color", |_, this, color: LuaColor| {
            borrow_mut(&this.bridge)?.commands.push(UiCommand::SetColor(
                this.id.clone(),
                color.r,
                color.g,
                color.b,
                color.a,
            ));
            Ok(())
        });
        crate::bindings::register_generated_ui_methods(m);
        m.add_method("text", |_, this, ()| {
            Ok(borrow(&this.bridge)?
                .ui_text
                .get(&this.id)
                .cloned()
                .unwrap_or_default())
        });
        m.add_method("on_click", |lua, this, callback: mlua::Function| {
            let t: mlua::Table = match lua.globals().get("__fwok_ui_clicks") {
                Ok(table) => table,
                Err(_) => {
                    let table = lua.create_table()?;
                    lua.globals().set("__fwok_ui_clicks", table.clone())?;
                    table
                }
            };
            let entry = lua.create_table()?;
            entry.set("callback", callback)?;
            if let Some(scope) = current_scene_scope(lua)? {
                let scope_table = lua.create_table()?;
                scope_table.set("index", scope.index)?;
                scope_table.set("generation", scope.generation)?;
                entry.set("scope", scope_table)?;
            }
            t.set(this.id.as_str(), entry)
        });
    }
}

pub struct Api {
    pub bridge: BridgeRef,
}
impl crate::LuaGameApi for Api {
    fn register(&self, lua: &Lua) -> mlua::Result<()> {
        register_value_types(lua)?;
        let log = lua.create_table()?;
        for (name, level) in [
            ("info", LuaLogLevel::Info),
            ("warn", LuaLogLevel::Warn),
            ("error", LuaLogLevel::Error),
        ] {
            log.set(
                name,
                lua.create_function(move |_, msg: String| {
                    let message = format!("[LuaScript] {msg}");
                    match level {
                        LuaLogLevel::Info => fyrox::core::log::Log::info(message),
                        LuaLogLevel::Warn => fyrox::core::log::Log::warn(message),
                        LuaLogLevel::Error => fyrox::core::log::Log::err(message),
                    }
                    Ok(())
                })?,
            )?;
        }
        lua.globals().set("log", log)?;
        #[cfg(feature = "benchmark")]
        {
            let benchmark = lua.create_table()?;
            benchmark.set(
                "log",
                lua.create_function(move |_, message: String| {
                    fyrox::core::log::Log::info(format!("[LuaBenchmark] {message}"));
                    Ok(())
                })?,
            )?;
            let benchmark_origin = Instant::now();
            benchmark.set(
                "clock",
                lua.create_function(move |_, ()| Ok(benchmark_origin.elapsed().as_secs_f64()))?,
            )?;
            benchmark.set(
                "rust_baseline",
                lua.create_function(|lua, ()| {
                    let report = crate::run_rust_baseline();
                    let result = lua.create_table()?;
                    for (name, value) in [
                        ("event", report.event),
                        ("transform", report.transform),
                        ("numeric", report.numeric),
                        ("text", report.text),
                        ("widgets", report.widgets),
                    ] {
                        let metric = lua.create_table()?;
                        metric.set("iterations", value.iterations)?;
                        metric.set("total_ns", value.elapsed.as_nanos() as u64)?;
                        metric.set("average_ns", value.average_ns())?;
                        metric.set("checksum", value.checksum)?;
                        result.set(name, metric)?;
                    }
                    Ok(result)
                })?,
            )?;
            lua.globals().set("benchmark", benchmark)?;
        }
        let ui = lua.create_table()?;
        let bridge = self.bridge.clone();
        let load_bridge = bridge.clone();
        ui.set(
            "load",
            lua.create_function(move |_, path: String| {
                if path.trim().is_empty() {
                    return Err(mlua::Error::runtime("ui.load requires a non-empty path"));
                }
                borrow_mut(&load_bridge)?
                    .commands
                    .push(UiCommand::Load(path));
                Ok(())
            })?,
        )?;
        let show_bridge = bridge.clone();
        ui.set(
            "show",
            lua.create_function(move |_, visible: Option<bool>| {
                borrow_mut(&show_bridge)?
                    .commands
                    .push(UiCommand::Show(visible.unwrap_or(true)));
                Ok(())
            })?,
        )?;
        let component = lua.create_function(move |lua, id: String| {
            let cache: mlua::Table = match lua.globals().get("__fwok_ui_components") {
                Ok(table) => table,
                Err(_) => {
                    let table = lua.create_table()?;
                    lua.globals().set("__fwok_ui_components", table.clone())?;
                    table
                }
            };
            if let Ok(existing) = cache.get::<mlua::AnyUserData>(id.as_str()) {
                return Ok(existing);
            }
            let proxy = lua.create_userdata(UiComponentRef {
                id: id.clone(),
                bridge: bridge.clone(),
            })?;
            borrow_mut(&bridge)?
                .commands
                .push(UiCommand::Resolve(id.clone()));
            cache.set(id, proxy.clone())?;
            Ok(proxy)
        })?;
        ui.set("component", component.clone())?;
        ui.set("find", component.clone())?;
        ui.set("register", component)?;
        for (name, kind) in [
            ("create_text", UiElementKind::Text),
            ("create_text_box", UiElementKind::TextBox),
            ("create_button", UiElementKind::Button),
        ] {
            let bridge = self.bridge.clone();
            ui.set(name, lua.create_function(move |_, (id, text, x, y, width, height): (String, String, f32, f32, f32, f32)| {
                let kind = match kind { UiElementKind::Text => UiElementKind::Text, UiElementKind::TextBox => UiElementKind::TextBox, UiElementKind::Button => UiElementKind::Button };
                borrow_mut(&bridge)?.commands.push(UiCommand::Create(UiElementSpec { kind, id, text, x, y, width, height }));
                Ok(())
            })?)?;
        }
        lua.globals().set("ui", ui)?;
        let scene = lua.create_table()?;
        let bridge = self.bridge.clone();
        let make_find = |global: bool, lua: &Lua, bridge: BridgeRef| {
            lua.create_function(move |lua, name: String| {
                let scope = if global {
                    None
                } else {
                    current_scene_scope(lua)?
                };
                let cache: mlua::Table = match lua.globals().get("__fwok_scene_nodes") {
                    Ok(table) => table,
                    Err(_) => {
                        let table = lua.create_table()?;
                        lua.globals().set("__fwok_scene_nodes", table.clone())?;
                        table
                    }
                };
                let key = format!("{}:{}", scope_key(scope), name);
                if let Ok(existing) = cache.get::<mlua::AnyUserData>(key.as_str()) {
                    return Ok(existing);
                }
                let proxy = lua.create_userdata(SceneNodeRef {
                    target: ScopedNodeName {
                        scope,
                        name: name.clone(),
                    },
                    bridge: bridge.clone(),
                })?;
                borrow_mut(&bridge)?
                    .scene_commands
                    .push(SceneCommand::Resolve(ScopedNodeName {
                        scope,
                        name: name.clone(),
                    }));
                cache.set(key, proxy.clone())?;
                Ok(proxy)
            })
        };
        scene.set("find", make_find(false, lua, bridge.clone())?)?;
        scene.set("global_find", make_find(true, lua, bridge)?)?;
        lua.globals().set("scene", scene)?;
        Ok(())
    }
}

#[inline]
fn scope_key(scope: Option<HandleToken>) -> String {
    match scope {
        Some(token) => format!("{}:{}", token.index, token.generation),
        None => "global".to_owned(),
    }
}

#[inline]
fn current_scene_scope(lua: &Lua) -> mlua::Result<Option<HandleToken>> {
    let value: mlua::Value = lua.globals().get("__fwok_current_scene_scope")?;
    match value {
        mlua::Value::Table(table) => Ok(Some(HandleToken {
            index: table.get("index")?,
            generation: table.get("generation")?,
        })),
        _ => Ok(None),
    }
}
