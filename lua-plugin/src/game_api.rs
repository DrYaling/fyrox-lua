use crate::handles::HandleToken;
#[cfg(feature = "benchmark")]
use fyrox::core::instant::Instant;
use mlua::{FromLua, Lua, UserData, UserDataFields, UserDataMethods, Value};
use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default)]
pub struct LuaVector2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LuaVector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LuaVector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Clone, Copy, Debug, Default)]
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
                    Value::Table(table) => {
                        let result = Self {
                            $($field: table.raw_get(stringify!($field))?),+
                        };
                        if false $(|| !result.$field.is_finite())+ {
                            return Err(mlua::Error::runtime(concat!("non-finite ", $lua_name, " field")));
                        }
                        Ok(result)
                    },
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
    // Runtime initialization may also be reached through a game-owned API.
    // Keep registration idempotent so engine and game bindings can safely
    // compose without replacing constructors or emitting duplicate globals.
    let globals = lua.globals();
    if globals
        .get::<Option<bool>>("__fwok_value_types_registered")?
        .unwrap_or(false)
    {
        return Ok(());
    }
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
    crate::value_bindings::register(lua)?;
    globals.set("__fwok_value_types_registered", true)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeStats {
    /// Commands moved out of the Lua bridge during the current run.
    pub queued_commands: u64,
    /// Commands successfully handed to the engine during the current run.
    pub applied_commands: u64,
    /// Commands discarded because a resource was unavailable or a queue was full.
    pub dropped_commands: u64,
    /// Time spent applying commands, in nanoseconds, accumulated over the run.
    pub apply_duration_ns: u64,
}

#[derive(Default, Debug)]
pub struct Bridge {
    pub ui_text: HashMap<String, String>,
    /// 已发往 Fyrox 但尚未确认由 UI 消息队列消费的文本目标。
    pub(crate) ui_text_pending: HashMap<SharedName, u8>,
    /// 按写入顺序记录文本缓存键，用于限制动态节点名造成的长期占用。
    ui_text_order: VecDeque<SharedName>,
    ui_state: HashMap<SharedName, UiImmediateState>,
    ui_state_order: VecDeque<SharedName>,
    pub commands: Vec<UiCommand>,
    pub scene_commands: Vec<SceneCommand>,
    ui_command_cost: usize,
    scene_command_cost: usize,
    pub stats: BridgeStats,
}

/// Shared immutable identifier used by Lua proxies and queued commands.
/// Cloning it only increments an `Rc` counter instead of allocating a new
/// string for each high-frequency setter.
pub type SharedName = Rc<str>;

/// Lua-visible optimistic state for UI properties queued during the current
/// runtime. The host still applies the typed command to Fyrox later, while
/// reads in the same Lua turn observe the value just written.
#[derive(Clone, Debug, Default)]
pub(crate) struct UiImmediateState {
    pub text: Option<String>,
    pub position: Option<LuaVector2>,
    pub color: Option<LuaColor>,
    pub opacity: Option<f32>,
    pub visible: Option<bool>,
    pub enabled: Option<bool>,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl Bridge {
    #[inline]
    pub fn stats(&self) -> BridgeStats {
        self.stats
    }

    #[inline]
    pub(crate) fn push_ui_command(&mut self, command: UiCommand) -> mlua::Result<()> {
        command.validate()?;
        ensure_command_capacity(self, command.work_cost())?;
        self.apply_ui_immediate(&command);
        self.ui_command_cost += command.work_cost();
        self.commands.push(command);
        Ok(())
    }

    #[inline]
    pub(crate) fn extend_ui_commands(&mut self, commands: Vec<UiCommand>) -> mlua::Result<()> {
        for command in &commands {
            command.validate()?;
        }
        let added_cost = commands.iter().map(UiCommand::work_cost).sum();
        ensure_command_capacity(self, added_cost)?;
        for command in &commands {
            self.apply_ui_immediate(command);
        }
        self.ui_command_cost += added_cost;
        self.commands.extend(commands);
        Ok(())
    }

    #[inline]
    pub(crate) fn push_scene_command(&mut self, command: SceneCommand) -> mlua::Result<()> {
        command.validate()?;
        ensure_command_capacity(self, command.work_cost())?;
        self.scene_command_cost += command.work_cost();
        self.scene_commands.push(command);
        Ok(())
    }

    #[inline]
    pub(crate) fn extend_scene_commands(
        &mut self,
        commands: Vec<SceneCommand>,
    ) -> mlua::Result<()> {
        for command in &commands {
            command.validate()?;
        }
        let added_cost = commands.iter().map(SceneCommand::work_cost).sum();
        ensure_command_capacity(self, added_cost)?;
        self.scene_command_cost += added_cost;
        self.scene_commands.extend(commands);
        Ok(())
    }

    #[inline]
    pub(crate) fn take_ui_commands(&mut self) -> Vec<UiCommand> {
        self.ui_command_cost = 0;
        std::mem::take(&mut self.commands)
    }

    #[inline]
    pub(crate) fn take_scene_commands(&mut self) -> Vec<SceneCommand> {
        self.scene_command_cost = 0;
        std::mem::take(&mut self.scene_commands)
    }

    #[inline]
    pub(crate) fn clear_commands(&mut self) {
        self.commands.clear();
        self.scene_commands.clear();
        self.ui_command_cost = 0;
        self.scene_command_cost = 0;
        self.ui_text.clear();
        self.ui_text_pending.clear();
        self.ui_text_order.clear();
        self.clear_all_ui_state();
    }

    #[inline]
    fn queued_command_cost(&self) -> usize {
        self.ui_command_cost + self.scene_command_cost
    }

    #[inline]
    pub(crate) fn apply_ui_immediate(&mut self, command: &UiCommand) {
        let (id, update) = match command {
            UiCommand::SetText(id, value) => (id, UiImmediateUpdate::Text(value.clone())),
            UiCommand::Append(id, value) => (id, UiImmediateUpdate::Append(value.clone())),
            UiCommand::SetLayout(id, x, y, width, height) => {
                (id, UiImmediateUpdate::Layout(*x, *y, *width, *height))
            }
            UiCommand::SetTint(id, r, g, b, a, opacity) => {
                (id, UiImmediateUpdate::Tint(*r, *g, *b, *a, *opacity))
            }
            UiCommand::SetVisible(id, value) => (id, UiImmediateUpdate::Visible(*value)),
            UiCommand::SetEnabled(id, value) => (id, UiImmediateUpdate::Enabled(*value)),
            UiCommand::SetWidth(id, value) => (id, UiImmediateUpdate::Width(*value)),
            UiCommand::SetHeight(id, value) => (id, UiImmediateUpdate::Height(*value)),
            UiCommand::SetPosition(id, x, y) => (id, UiImmediateUpdate::Position(*x, *y)),
            UiCommand::SetOpacity(id, value) => (id, UiImmediateUpdate::Opacity(*value)),
            UiCommand::SetColor(id, r, g, b, a) => (id, UiImmediateUpdate::Color(*r, *g, *b, *a)),
            _ => return,
        };
        let append_fallback = match &update {
            UiImmediateUpdate::Append(_) => self.ui_text.get(id.as_ref()).cloned(),
            _ => None,
        };
        if !self.ui_state.contains_key(id.as_ref()) {
            if self.ui_state.len() >= MAX_UI_IMMEDIATE_STATES {
                if let Some(evicted) = self.ui_state_order.pop_front() {
                    self.ui_state.remove(&evicted);
                }
            }
            self.ui_state_order.push_back(id.clone());
        }
        let state = self.ui_state.entry(id.clone()).or_default();
        match update {
            UiImmediateUpdate::Text(value) => state.text = Some(value),
            UiImmediateUpdate::Append(value) => {
                let current = state
                    .text
                    .get_or_insert(append_fallback.unwrap_or_default());
                current.push_str(&value);
            }
            UiImmediateUpdate::Layout(x, y, width, height) => {
                state.position = Some(LuaVector2 { x, y });
                state.width = Some(width);
                state.height = Some(height);
            }
            UiImmediateUpdate::Tint(r, g, b, a, opacity) => {
                state.color = Some(LuaColor { r, g, b, a });
                state.opacity = Some(opacity);
            }
            UiImmediateUpdate::Visible(value) => state.visible = Some(value),
            UiImmediateUpdate::Enabled(value) => state.enabled = Some(value),
            UiImmediateUpdate::Width(value) => state.width = Some(value),
            UiImmediateUpdate::Height(value) => state.height = Some(value),
            UiImmediateUpdate::Position(x, y) => state.position = Some(LuaVector2 { x, y }),
            UiImmediateUpdate::Opacity(value) => state.opacity = Some(value),
            UiImmediateUpdate::Color(r, g, b, a) => state.color = Some(LuaColor { r, g, b, a }),
        }
    }

    #[inline]
    pub(crate) fn ui_state(&self, id: &str) -> Option<&UiImmediateState> {
        self.ui_state.get(id)
    }

    /// Commits text commands into the read cache and retires their optimistic
    /// shadow values after the host has accepted the commands for application.
    #[inline]
    pub(crate) fn commit_ui_text_commands(&mut self, commands: &[UiCommand]) {
        for command in commands {
            match command {
                UiCommand::SetText(id, value) => {
                    self.set_ui_text_value(id, value.clone(), true);
                    self.ui_text_pending.insert(id.clone(), 0);
                    if let Some(state) = self.ui_state.get_mut(id.as_ref()) {
                        state.text = None;
                    }
                }
                UiCommand::Append(id, value) => {
                    let mut text = self.ui_text.get(id.as_ref()).cloned().unwrap_or_default();
                    text.push_str(value);
                    self.set_ui_text_value(id, text, true);
                    self.ui_text_pending.insert(id.clone(), 0);
                    if let Some(state) = self.ui_state.get_mut(id.as_ref()) {
                        state.text = None;
                    }
                }
                _ => {}
            }
        }
        self.ui_state.retain(|_, state| {
            state.text.is_some()
                || state.position.is_some()
                || state.color.is_some()
                || state.opacity.is_some()
                || state.visible.is_some()
                || state.enabled.is_some()
                || state.width.is_some()
                || state.height.is_some()
        });
        self.ui_state_order
            .retain(|id| self.ui_state.contains_key(id.as_ref()));
    }

    /// Updates the bounded committed text cache and its ordered eviction queue.
    #[inline]
    fn set_ui_text_value(&mut self, id: &SharedName, value: String, pending: bool) {
        if !self.ui_text.contains_key(id.as_ref()) {
            while self.ui_text.len() >= MAX_UI_TEXT_CACHE {
                let Some(oldest) = self.ui_text_order.pop_front() else {
                    break;
                };
                self.ui_text.remove(oldest.as_ref());
                self.ui_text_pending.remove(oldest.as_ref());
            }
            self.ui_text_order.push_back(id.clone());
        }
        self.ui_text.insert(id.to_string(), value);
        if pending {
            self.ui_text_pending.insert(id.clone(), 0);
        }
    }

    /// Synchronizes an engine-observed value without overwriting an unconsumed Lua update.
    #[inline]
    pub(crate) fn sync_ui_text_value(&mut self, id: &str, value: String) {
        if let Some(misses) = self.ui_text_pending.get_mut(id) {
            if self.ui_text.get(id) == Some(&value) {
                self.ui_text_pending.remove(id);
            } else if *misses >= MAX_PENDING_TEXT_SYNC_MISSES {
                self.ui_text_pending.remove(id);
                let id = Rc::<str>::from(id);
                self.set_ui_text_value(&id, value, false);
            } else {
                *misses += 1;
            }
            return;
        }
        let id = Rc::<str>::from(id);
        self.set_ui_text_value(&id, value, false);
    }

    #[inline]
    pub(crate) fn clear_all_ui_state(&mut self) {
        self.ui_state.clear();
        self.ui_state_order.clear();
    }

    #[inline]
    pub(crate) fn rebuild_ui_state<'a>(
        &mut self,
        commands: impl IntoIterator<Item = &'a UiCommand>,
    ) {
        self.clear_all_ui_state();
        for command in commands {
            self.apply_ui_immediate(command);
        }
    }
}

#[derive(Debug)]
enum UiImmediateUpdate {
    Text(String),
    Append(String),
    Layout(f32, f32, f32, f32),
    Tint(f32, f32, f32, f32, f32),
    Visible(bool),
    Enabled(bool),
    Width(f32),
    Height(f32),
    Position(f32, f32),
    Opacity(f32),
    Color(f32, f32, f32, f32),
}

pub(crate) const MAX_COMMANDS_PER_FRAME: usize = 20_000;
const MAX_UI_IMMEDIATE_STATES: usize = 4096;
/// Maximum committed UI text entries retained between frames.
pub(crate) const MAX_UI_TEXT_CACHE: usize = 4096;
/// Number of host sync passes tolerated while an asynchronous text message is pending.
const MAX_PENDING_TEXT_SYNC_MISSES: u8 = 4;

#[derive(Debug, Clone)]
pub enum SceneCommand {
    Resolve(ScopedNodeName),
    SetTransform(ScopedNodeName, LuaVector3, LuaVector3, LuaVector3),
    SetPosition(ScopedNodeName, f32, f32, f32),
    SetRotationZ(ScopedNodeName, f32),
    SetRotationAngles(ScopedNodeName, f32, f32, f32),
    SetScale(ScopedNodeName, f32, f32, f32),
    SetEnabled(ScopedNodeName, bool),
}

impl SceneCommand {
    #[inline]
    fn work_cost(&self) -> usize {
        match self {
            Self::SetTransform(..) => 3,
            _ => 1,
        }
    }

    #[inline]
    fn validate(&self) -> mlua::Result<()> {
        let valid = match self {
            Self::SetTransform(_, position, rotation, scale) => {
                position.x.is_finite()
                    && position.y.is_finite()
                    && position.z.is_finite()
                    && rotation.x.is_finite()
                    && rotation.y.is_finite()
                    && rotation.z.is_finite()
                    && scale.x.is_finite()
                    && scale.y.is_finite()
                    && scale.z.is_finite()
            }
            Self::SetPosition(_, x, y, z)
            | Self::SetRotationAngles(_, x, y, z)
            | Self::SetScale(_, x, y, z) => x.is_finite() && y.is_finite() && z.is_finite(),
            Self::SetRotationZ(_, angle) => angle.is_finite(),
            Self::Resolve(_) | Self::SetEnabled(_, _) => true,
        };
        if valid {
            Ok(())
        } else {
            Err(mlua::Error::runtime(
                "scene command contains non-finite numeric value",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopedNodeName {
    pub scope: Option<HandleToken>,
    pub name: SharedName,
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
    Resolve(SharedName),
    ResolveRequired(SharedName),
    Create(UiElementSpec),
    SetText(SharedName, String),
    SetLayout(SharedName, f32, f32, f32, f32),
    SetTint(SharedName, f32, f32, f32, f32, f32),
    Append(SharedName, String),
    SetVisible(SharedName, bool),
    SetEnabled(SharedName, bool),
    SetWidth(SharedName, f32),
    SetHeight(SharedName, f32),
    SetPosition(SharedName, f32, f32),
    SetChecked(SharedName, bool),
    SetSelected(SharedName, Option<usize>),
    SetScroll(SharedName, f32, f32),
    SetProgress(SharedName, f32),
    SetPopupOpen(SharedName, bool),
    SetOpacity(SharedName, f32),
    SetGridRow(SharedName, usize),
    SetGridColumn(SharedName, usize),
    SetColor(SharedName, f32, f32, f32, f32),
    Log(LuaLogLevel, String),
}

impl UiCommand {
    #[inline]
    pub(crate) fn work_cost(&self) -> usize {
        match self {
            Self::SetLayout(..) => 3,
            Self::SetTint(..) => 2,
            _ => 1,
        }
    }

    #[inline]
    fn validate(&self) -> mlua::Result<()> {
        let valid = match self {
            Self::Create(spec) => {
                spec.x.is_finite()
                    && spec.y.is_finite()
                    && spec.width.is_finite()
                    && spec.height.is_finite()
            }
            Self::SetLayout(_, x, y, width, height) => {
                x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite()
            }
            Self::SetTint(_, r, g, b, a, opacity) => {
                r.is_finite()
                    && g.is_finite()
                    && b.is_finite()
                    && a.is_finite()
                    && opacity.is_finite()
            }
            Self::SetPosition(_, x, y) => x.is_finite() && y.is_finite(),
            Self::SetScroll(_, x, y) => x.is_finite() && y.is_finite(),
            Self::SetProgress(_, value) | Self::SetOpacity(_, value) => value.is_finite(),
            Self::SetColor(_, r, g, b, a) => {
                r.is_finite() && g.is_finite() && b.is_finite() && a.is_finite()
            }
            Self::SetWidth(_, value) | Self::SetHeight(_, value) => value.is_finite(),
            Self::Load(_)
            | Self::Show(_)
            | Self::Resolve(_)
            | Self::ResolveRequired(_)
            | Self::SetText(_, _)
            | Self::Append(_, _)
            | Self::SetVisible(_, _)
            | Self::SetEnabled(_, _)
            | Self::SetChecked(_, _)
            | Self::SetSelected(_, _)
            | Self::SetPopupOpen(_, _)
            | Self::SetGridRow(_, _)
            | Self::SetGridColumn(_, _)
            | Self::Log(_, _) => true,
        };
        if valid {
            Ok(())
        } else {
            Err(mlua::Error::runtime(
                "ui command contains non-finite numeric value",
            ))
        }
    }
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

pub(crate) fn borrow_mut(b: &BridgeRef) -> mlua::Result<RefMut<'_, Bridge>> {
    let bridge = b
        .try_borrow_mut()
        .map_err(|_| mlua::Error::runtime("Lua host context is already mutably borrowed"))?;
    if bridge.queued_command_cost() >= MAX_COMMANDS_PER_FRAME {
        return Err(mlua::Error::runtime(format!(
            "Lua command buffer limit exceeded ({MAX_COMMANDS_PER_FRAME})"
        )));
    }
    Ok(bridge)
}

#[inline]
pub(crate) fn ensure_command_capacity(bridge: &Bridge, additional: usize) -> mlua::Result<()> {
    if bridge.queued_command_cost() + additional > MAX_COMMANDS_PER_FRAME {
        return Err(mlua::Error::runtime(format!(
            "Lua command buffer limit exceeded ({MAX_COMMANDS_PER_FRAME})"
        )));
    }
    Ok(())
}

/// Creates a translator-style weak-value cache.  tolua's object translator
/// does not keep otherwise unreachable wrappers alive; matching that behavior
/// prevents dynamic UI/scene names from growing a permanent Lua root set.
#[inline]
fn weak_value_cache(lua: &Lua, global: &str) -> mlua::Result<mlua::Table> {
    if let Ok(table) = lua.globals().get::<mlua::Table>(global) {
        return Ok(table);
    }
    let table = lua.create_table()?;
    let metatable = lua.create_table()?;
    metatable.set("__mode", "v")?;
    table.set_metatable(Some(metatable))?;
    lua.globals().set(global, table.clone())?;
    Ok(table)
}

#[derive(Clone)]
pub(crate) struct UiComponentRef {
    pub(crate) id: SharedName,
    bridge: BridgeRef,
}

#[inline]
fn read_ui_text(lua: &Lua, this: &UiComponentRef) -> mlua::Result<mlua::LuaString> {
    let bridge = this
        .bridge
        .try_borrow()
        .map_err(|_| mlua::Error::runtime("Lua host context is already mutably borrowed"))?;
    if let Some(value) = bridge
        .ui_state(this.id.as_ref())
        .and_then(|state| state.text.as_ref())
    {
        return lua.create_string(value);
    }
    lua.create_string(
        bridge
            .ui_text
            .get(this.id.as_ref())
            .map(String::as_str)
            .unwrap_or_default(),
    )
}

#[inline]
pub(crate) fn queue_ui_command(this: &UiComponentRef, command: UiCommand) -> mlua::Result<()> {
    borrow_mut(&this.bridge)?.push_ui_command(command)
}

#[derive(Clone)]
struct SceneNodeRef {
    target: ScopedNodeName,
    bridge: BridgeRef,
}

impl UserData for SceneNodeRef {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method(
            "set_transform",
            |_, this, (position, rotation, scale): (LuaVector3, LuaVector3, LuaVector3)| {
                let mut bridge = borrow_mut(&this.bridge)?;
                bridge.push_scene_command(SceneCommand::SetTransform(
                    this.target.clone(),
                    position,
                    rotation,
                    scale,
                ))
            },
        );
        m.add_method("set_position", |lua, this, values: mlua::MultiValue| {
            let LuaVector3 { x, y, z } = vector3_args(lua, values)?;
            borrow_mut(&this.bridge)?.push_scene_command(SceneCommand::SetPosition(
                this.target.clone(),
                x,
                y,
                z,
            ))
        });
        m.add_method("set_rotation_z", |_, this, angle: f32| {
            borrow_mut(&this.bridge)?
                .push_scene_command(SceneCommand::SetRotationZ(this.target.clone(), angle))
        });
        m.add_method(
            "set_rotation",
            |_, this, (roll, pitch, yaw): (f32, f32, f32)| {
                borrow_mut(&this.bridge)?.push_scene_command(SceneCommand::SetRotationAngles(
                    this.target.clone(),
                    roll,
                    pitch,
                    yaw,
                ))
            },
        );
        m.add_method("set_scale", |lua, this, values: mlua::MultiValue| {
            let LuaVector3 { x, y, z } = vector3_args(lua, values)?;
            borrow_mut(&this.bridge)?.push_scene_command(SceneCommand::SetScale(
                this.target.clone(),
                x,
                y,
                z,
            ))
        });
        m.add_method("set_enabled", |_, this, value: bool| {
            borrow_mut(&this.bridge)?
                .push_scene_command(SceneCommand::SetEnabled(this.target.clone(), value))
        });
    }
}

#[inline]
fn vector3_args(lua: &Lua, values: mlua::MultiValue) -> mlua::Result<LuaVector3> {
    let mut values = values.into_iter();
    let first = values.next().unwrap_or(Value::Nil);
    let second = values.next();
    let third = values.next();
    if second.is_none() && third.is_none() {
        return LuaVector3::from_lua(first, lua);
    }
    if let (Some(second), Some(third)) = (second, third) {
        if values.next().is_none() {
            return Ok(LuaVector3 {
                x: f32::from_lua(first, lua)?,
                y: f32::from_lua(second, lua)?,
                z: f32::from_lua(third, lua)?,
            });
        }
    }
    Err(mlua::Error::runtime(
        "expected Vector3 or three numeric arguments",
    ))
}

#[inline]
fn vector2_args(lua: &Lua, values: mlua::MultiValue) -> mlua::Result<LuaVector2> {
    let mut values = values.into_iter();
    let first = values.next().unwrap_or(Value::Nil);
    let second = values.next();
    if let Some(second) = second {
        if values.next().is_none() {
            return Ok(LuaVector2 {
                x: f32::from_lua(first, lua)?,
                y: f32::from_lua(second, lua)?,
            });
        }
    } else {
        return LuaVector2::from_lua(first, lua);
    }
    Err(mlua::Error::runtime(
        "expected Vector2 or two numeric arguments",
    ))
}
impl UserData for UiComponentRef {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("text", |lua, this| read_ui_text(lua, this));
        fields.add_field_method_set("text", |_, this, value: String| {
            queue_ui_command(this, UiCommand::SetText(this.id.clone(), value))
        });
        fields.add_field_method_get("position", |_, this| {
            let bridge = this.bridge.try_borrow().map_err(|_| {
                mlua::Error::runtime("Lua host context is already mutably borrowed")
            })?;
            Ok(bridge
                .ui_state(this.id.as_ref())
                .and_then(|state| state.position))
        });
        fields.add_field_method_set("position", |_, this, value: LuaVector2| {
            queue_ui_command(
                this,
                UiCommand::SetPosition(this.id.clone(), value.x, value.y),
            )
        });
        fields.add_field_method_get("color", |_, this| {
            let bridge = this.bridge.try_borrow().map_err(|_| {
                mlua::Error::runtime("Lua host context is already mutably borrowed")
            })?;
            Ok(bridge
                .ui_state(this.id.as_ref())
                .and_then(|state| state.color))
        });
        fields.add_field_method_set("color", |_, this, value: LuaColor| {
            queue_ui_command(
                this,
                UiCommand::SetColor(this.id.clone(), value.r, value.g, value.b, value.a),
            )
        });
        for (name, getter) in [("opacity", 0usize), ("width", 1usize), ("height", 2usize)] {
            fields.add_field_method_get(name, move |_, this| {
                let bridge = this.bridge.try_borrow().map_err(|_| {
                    mlua::Error::runtime("Lua host context is already mutably borrowed")
                })?;
                Ok(bridge
                    .ui_state(this.id.as_ref())
                    .and_then(|state| match getter {
                        0 => state.opacity,
                        1 => state.width,
                        _ => state.height,
                    }))
            });
        }
        fields.add_field_method_set("opacity", |_, this, value: f32| {
            queue_ui_command(this, UiCommand::SetOpacity(this.id.clone(), value))
        });
        fields.add_field_method_set("width", |_, this, value: f32| {
            queue_ui_command(this, UiCommand::SetWidth(this.id.clone(), value))
        });
        fields.add_field_method_set("height", |_, this, value: f32| {
            queue_ui_command(this, UiCommand::SetHeight(this.id.clone(), value))
        });
        for (name, getter) in [("visible", true), ("enabled", false)] {
            fields.add_field_method_get(name, move |_, this| {
                let bridge = this.bridge.try_borrow().map_err(|_| {
                    mlua::Error::runtime("Lua host context is already mutably borrowed")
                })?;
                Ok(bridge.ui_state(this.id.as_ref()).and_then(|state| {
                    if getter {
                        state.visible
                    } else {
                        state.enabled
                    }
                }))
            });
        }
        fields.add_field_method_set("visible", |_, this, value: bool| {
            queue_ui_command(this, UiCommand::SetVisible(this.id.clone(), value))
        });
        fields.add_field_method_set("enabled", |_, this, value: bool| {
            queue_ui_command(this, UiCommand::SetEnabled(this.id.clone(), value))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method(
            "set_layout",
            |_, this, (x, y, width, height): (f32, f32, f32, f32)| {
                borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetLayout(
                    this.id.clone(),
                    x,
                    y,
                    width,
                    height,
                ))
            },
        );
        m.add_method(
            "set_tint",
            |_, this, (r, g, b, a, opacity): (f32, f32, f32, f32, f32)| {
                borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetTint(
                    this.id.clone(),
                    r,
                    g,
                    b,
                    a,
                    opacity,
                ))
            },
        );
        m.add_method("set_text", |_, this, value: String| {
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetText(this.id.clone(), value))
        });
        m.add_method("append", |_, this, value: String| {
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::Append(this.id.clone(), value))
        });
        m.add_method("set_visible", |_, this, value: bool| {
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetVisible(this.id.clone(), value))
        });
        m.add_method("set_enabled", |_, this, value: bool| {
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetEnabled(this.id.clone(), value))
        });
        m.add_method("set_width", |_, this, value: f32| {
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetWidth(this.id.clone(), value))
        });
        m.add_method("set_height", |_, this, value: f32| {
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetHeight(this.id.clone(), value))
        });
        m.add_method("set_position", |lua, this, values: mlua::MultiValue| {
            let LuaVector2 { x, y } = vector2_args(lua, values)?;
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetPosition(this.id.clone(), x, y))
        });
        m.add_method("set_color", |_, this, color: LuaColor| {
            borrow_mut(&this.bridge)?.push_ui_command(UiCommand::SetColor(
                this.id.clone(),
                color.r,
                color.g,
                color.b,
                color.a,
            ))
        });
        crate::bindings::register_generated_ui_methods(m);
        m.add_method("get_text", |lua, this, ()| read_ui_text(lua, this));
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
            t.set(this.id.as_ref(), entry)
        });
        m.add_method("off_click", |lua, this, ()| {
            if let Ok(callbacks) = lua.globals().get::<mlua::Table>("__fwok_ui_clicks") {
                callbacks.raw_set(this.id.as_ref(), Value::Nil)?;
            }
            Ok(())
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
                        ("ui_layout", crate::run_rust_ui_layout(40)),
                        ("ui_layout_legacy", crate::run_rust_ui_layout_legacy(40)),
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
                borrow_mut(&load_bridge)?.push_ui_command(UiCommand::Load(path))
            })?,
        )?;
        let show_bridge = bridge.clone();
        ui.set(
            "show",
            lua.create_function(move |_, visible: Option<bool>| {
                borrow_mut(&show_bridge)?.push_ui_command(UiCommand::Show(visible.unwrap_or(true)))
            })?,
        )?;
        let make_component = |required: bool, bridge: BridgeRef| {
            lua.create_function(move |lua, id: String| {
                if id.trim().is_empty() {
                    return Err(mlua::Error::runtime("ui component name must not be empty"));
                }
                let cache = weak_value_cache(lua, "__fwok_ui_components")?;
                if let Ok(existing) = cache.raw_get::<mlua::AnyUserData>(id.as_str()) {
                    if required {
                        borrow_mut(&bridge)?
                            .push_ui_command(UiCommand::ResolveRequired(Rc::from(id.as_str())))?;
                    }
                    return Ok(existing);
                }
                let proxy = lua.create_userdata(UiComponentRef {
                    id: Rc::from(id.as_str()),
                    bridge: bridge.clone(),
                })?;
                borrow_mut(&bridge)?.push_ui_command(if required {
                    UiCommand::ResolveRequired(Rc::from(id.as_str()))
                } else {
                    UiCommand::Resolve(Rc::from(id.as_str()))
                })?;
                cache.set(id, proxy.clone())?;
                Ok(proxy)
            })
        };
        let component = make_component(false, bridge.clone())?;
        let required = make_component(true, bridge)?;
        ui.set("component", component.clone())?;
        ui.set("find", component.clone())?;
        ui.set("register", component)?;
        ui.set("require", required)?;
        for (name, kind) in [
            ("create_text", UiElementKind::Text),
            ("create_text_box", UiElementKind::TextBox),
            ("create_button", UiElementKind::Button),
        ] {
            let bridge = self.bridge.clone();
            ui.set(name, lua.create_function(move |_, (id, text, x, y, width, height): (String, String, f32, f32, f32, f32)| {
                let kind = match kind { UiElementKind::Text => UiElementKind::Text, UiElementKind::TextBox => UiElementKind::TextBox, UiElementKind::Button => UiElementKind::Button };
                borrow_mut(&bridge)?.push_ui_command(UiCommand::Create(UiElementSpec { kind, id, text, x, y, width, height }))
            })?)?;
        }
        ui.set(
            "batch",
            crate::batch_api::register_ui(lua, self.bridge.clone())?,
        )?;
        ui.set(
            "batch_compact",
            crate::batch_api::register_ui_compact(lua, self.bridge.clone())?,
        )?;
        ui.set(
            "batch_ops",
            crate::batch_api::register_ui_ops(lua, self.bridge.clone())?,
        )?;
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
                let cache = weak_value_cache(lua, "__fwok_scene_nodes")?;
                let key = format!("{}:{}", scope_key(scope), name);
                if let Ok(existing) = cache.raw_get::<mlua::AnyUserData>(key.as_str()) {
                    return Ok(existing);
                }
                let proxy = lua.create_userdata(SceneNodeRef {
                    target: ScopedNodeName {
                        scope,
                        name: Rc::from(name.as_str()),
                    },
                    bridge: bridge.clone(),
                })?;
                borrow_mut(&bridge)?.push_scene_command(SceneCommand::Resolve(ScopedNodeName {
                    scope,
                    name: Rc::from(name.as_str()),
                }))?;
                cache.set(key, proxy.clone())?;
                Ok(proxy)
            })
        };
        scene.set("find", make_find(false, lua, bridge.clone())?)?;
        scene.set("global_find", make_find(true, lua, bridge)?)?;
        scene.set(
            "batch",
            crate::batch_api::register_scene(lua, self.bridge.clone())?,
        )?;
        scene.set(
            "batch_compact",
            crate::batch_api::register_scene_compact(lua, self.bridge.clone())?,
        )?;
        scene.set(
            "batch_ops",
            crate::batch_api::register_scene_ops(lua, self.bridge.clone())?,
        )?;
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
pub(crate) fn current_scene_scope(lua: &Lua) -> mlua::Result<Option<HandleToken>> {
    let value: mlua::Value = lua.globals().get("__fwok_current_scene_scope")?;
    match value {
        mlua::Value::Table(table) => Ok(Some(HandleToken {
            index: table.get("index")?,
            generation: table.get("generation")?,
        })),
        _ => Ok(None),
    }
}
