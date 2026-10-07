//! High-frequency UI and scene command adapters.
use crate::game_api::{
    borrow_mut, current_scene_scope, BridgeRef, LuaVector3, SceneCommand, ScopedNodeName,
    UiCommand, MAX_COMMANDS_PER_FRAME,
};
use mlua::{Function, Lua, Table, Value};
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Default)]
struct NameCache {
    first: Option<Rc<str>>,
    extra: Option<HashSet<Rc<str>>>,
}

/// Reuse target names within one batch without retaining a process-wide name
/// interner. This removes repeated `Rc<str>` allocations for hot batches while
/// keeping dynamic IDs bounded by the lifetime of the current parser call.
#[inline]
fn cached_name(cache: &mut NameCache, value: mlua::LuaString) -> mlua::Result<Rc<str>> {
    let text = value.to_str()?;
    if let Some(first) = cache.first.as_ref() {
        if first.as_ref() == text.as_ref() {
            return Ok(first.clone());
        }
    } else {
        let name: Rc<str> = Rc::from(text.as_ref());
        cache.first = Some(name.clone());
        return Ok(name);
    }
    if let Some(extra) = cache.extra.as_ref() {
        if let Some(existing) = extra.get(text.as_ref()) {
            return Ok(existing.clone());
        }
    }
    let name: Rc<str> = Rc::from(text.as_ref());
    if let Some(extra) = cache.extra.as_mut() {
        extra.insert(name.clone());
    } else {
        let mut extra = HashSet::with_capacity(4);
        if let Some(first) = cache.first.as_ref() {
            extra.insert(first.clone());
        }
        extra.insert(name.clone());
        cache.extra = Some(extra);
    }
    Ok(name)
}

#[inline]
fn opcode_number(value: Value, max: u8, label: &str) -> mlua::Result<u8> {
    let number = match value {
        Value::Integer(value) => value as f64,
        Value::Number(value) => value,
        Value::String(value) => {
            return Err(mlua::Error::runtime(format!(
                "unsupported {label} op: {}",
                value.to_str()?.as_ref()
            )))
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "invalid {label} opcode value ({})",
                other.type_name()
            )))
        }
    };
    if !number.is_finite() || number.fract() != 0.0 || number < 1.0 || number > max as f64 {
        return Err(mlua::Error::runtime(format!(
            "invalid {label} opcode {number} (expected 1..={max})"
        )));
    }
    Ok(number as u8)
}

#[inline]
fn ui_opcode(value: Value) -> mlua::Result<u8> {
    match value {
        Value::String(value) => match value.to_str()?.as_ref() {
            "set_text" => Ok(1),
            "set_visible" => Ok(2),
            "set_enabled" => Ok(3),
            "set_progress" => Ok(4),
            "set_opacity" => Ok(5),
            "set_position" => Ok(6),
            "set_layout" => Ok(7),
            "set_tint" => Ok(8),
            op => Err(mlua::Error::runtime(format!(
                "unsupported ui.batch op: {op}"
            ))),
        },
        value => opcode_number(value, 8, "ui.batch"),
    }
}

#[inline]
fn scene_opcode(value: Value) -> mlua::Result<u8> {
    match value {
        Value::String(value) => match value.to_str()?.as_ref() {
            "set_position" => Ok(1),
            "set_rotation" => Ok(2),
            "set_scale" => Ok(3),
            "set_enabled" => Ok(4),
            "set_transform" => Ok(5),
            op => Err(mlua::Error::runtime(format!(
                "unsupported scene.batch op: {op}"
            ))),
        },
        value => opcode_number(value, 5, "scene.batch"),
    }
}

#[inline]
fn ui_command_for(op: u8, id: Rc<str>, item: &Table, offset: i64) -> mlua::Result<UiCommand> {
    Ok(match op {
        1 => UiCommand::SetText(id, item.raw_get(offset + 0)?),
        2 => UiCommand::SetVisible(id, item.raw_get(offset + 0)?),
        3 => UiCommand::SetEnabled(id, item.raw_get(offset + 0)?),
        4 => UiCommand::SetProgress(id, item.raw_get(offset + 0)?),
        5 => UiCommand::SetOpacity(id, item.raw_get(offset + 0)?),
        6 => UiCommand::SetPosition(id, item.raw_get(offset + 0)?, item.raw_get(offset + 1)?),
        7 => UiCommand::SetLayout(
            id,
            item.raw_get(offset + 0)?,
            item.raw_get(offset + 1)?,
            item.raw_get(offset + 2)?,
            item.raw_get(offset + 3)?,
        ),
        8 => UiCommand::SetTint(
            id,
            item.raw_get(offset + 0)?,
            item.raw_get(offset + 1)?,
            item.raw_get(offset + 2)?,
            item.raw_get(offset + 3)?,
            item.raw_get(offset + 4)?,
        ),
        _ => unreachable!("ui opcode was validated before command conversion"),
    })
}

#[inline]
fn ui_command_keyed(op: u8, id: Rc<str>, item: &Table) -> mlua::Result<UiCommand> {
    Ok(match op {
        1 => UiCommand::SetText(id, item.raw_get("value")?),
        2 => UiCommand::SetVisible(id, item.raw_get("value")?),
        3 => UiCommand::SetEnabled(id, item.raw_get("value")?),
        4 => UiCommand::SetProgress(id, item.raw_get("value")?),
        5 => UiCommand::SetOpacity(id, item.raw_get("value")?),
        6 => UiCommand::SetPosition(id, item.raw_get("x")?, item.raw_get("y")?),
        7 => UiCommand::SetLayout(
            id,
            item.raw_get("x")?,
            item.raw_get("y")?,
            item.raw_get("width")?,
            item.raw_get("height")?,
        ),
        8 => UiCommand::SetTint(
            id,
            item.raw_get("r")?,
            item.raw_get("g")?,
            item.raw_get("b")?,
            item.raw_get("a")?,
            item.raw_get("opacity")?,
        ),
        _ => unreachable!("ui opcode was validated before command conversion"),
    })
}

#[inline]
fn scene_command_for(
    op: u8,
    target: ScopedNodeName,
    item: &Table,
    offset: i64,
) -> mlua::Result<SceneCommand> {
    Ok(match op {
        5 => SceneCommand::SetTransform(
            target,
            LuaVector3 {
                x: item.raw_get(offset + 0)?,
                y: item.raw_get(offset + 1)?,
                z: item.raw_get(offset + 2)?,
            },
            LuaVector3 {
                x: item.raw_get(offset + 3)?,
                y: item.raw_get(offset + 4)?,
                z: item.raw_get(offset + 5)?,
            },
            LuaVector3 {
                x: item.raw_get(offset + 6)?,
                y: item.raw_get(offset + 7)?,
                z: item.raw_get(offset + 8)?,
            },
        ),
        1 => SceneCommand::SetPosition(
            target,
            item.raw_get(offset + 0)?,
            item.raw_get(offset + 1)?,
            item.raw_get(offset + 2)?,
        ),
        2 => SceneCommand::SetRotationAngles(
            target,
            item.raw_get(offset + 0)?,
            item.raw_get(offset + 1)?,
            item.raw_get(offset + 2)?,
        ),
        3 => SceneCommand::SetScale(
            target,
            item.raw_get(offset + 0)?,
            item.raw_get(offset + 1)?,
            item.raw_get(offset + 2)?,
        ),
        4 => SceneCommand::SetEnabled(target, item.raw_get(offset + 0)?),
        _ => unreachable!("scene opcode was validated before command conversion"),
    })
}

#[inline]
fn scene_command_keyed(op: u8, target: ScopedNodeName, item: &Table) -> mlua::Result<SceneCommand> {
    Ok(match op {
        1 => SceneCommand::SetPosition(
            target,
            item.raw_get("x")?,
            item.raw_get("y")?,
            item.raw_get("z")?,
        ),
        2 => SceneCommand::SetRotationAngles(
            target,
            item.raw_get("roll")?,
            item.raw_get("pitch")?,
            item.raw_get("yaw")?,
        ),
        3 => SceneCommand::SetScale(
            target,
            item.raw_get("x")?,
            item.raw_get("y")?,
            item.raw_get("z")?,
        ),
        4 => SceneCommand::SetEnabled(target, item.raw_get("value")?),
        5 => SceneCommand::SetTransform(
            target,
            LuaVector3 {
                x: item.raw_get("px")?,
                y: item.raw_get("py")?,
                z: item.raw_get("pz")?,
            },
            LuaVector3 {
                x: item.raw_get("roll")?,
                y: item.raw_get("pitch")?,
                z: item.raw_get("yaw")?,
            },
            LuaVector3 {
                x: item.raw_get("sx")?,
                y: item.raw_get("sy")?,
                z: item.raw_get("sz")?,
            },
        ),
        _ => unreachable!("scene opcode was validated before command conversion"),
    })
}

pub(crate) fn register_ui(lua: &Lua, bridge: BridgeRef) -> mlua::Result<Function> {
    lua.create_function(move |_, operations: Table| {
        let count = operations.raw_len();
        if count == 0 {
            return Ok(());
        }
        if count > MAX_COMMANDS_PER_FRAME {
            return Err(mlua::Error::runtime(format!(
                "ui.batch contains too many commands ({count}, limit {MAX_COMMANDS_PER_FRAME})"
            )));
        }
        let mut commands = Vec::with_capacity(count);
        let mut names = NameCache::default();
        for index in 1..=count {
            let item: Table = operations.raw_get(index)?;
            let op = ui_opcode(item.raw_get::<Value>("op")?)?;
            let id = cached_name(&mut names, item.raw_get("id")?)?;
            commands.push(ui_command_keyed(op, id, &item)?);
        }
        let mut bridge = borrow_mut(&bridge)?;
        bridge.extend_ui_commands(commands)
    })
}

/// Positional batch protocol. It avoids per-operation key strings and tables such as
/// `{op=..., id=..., value=...}` while retaining one command per array item.
pub(crate) fn register_ui_compact(lua: &Lua, bridge: BridgeRef) -> mlua::Result<Function> {
    lua.create_function(move |_, operations: Table| {
        let count = operations.raw_len();
        if count == 0 {
            return Ok(());
        }
        if count > MAX_COMMANDS_PER_FRAME {
            return Err(mlua::Error::runtime(format!(
                "ui.batch_compact contains too many commands ({count}, limit {MAX_COMMANDS_PER_FRAME})"
            )));
        }
        let mut commands = Vec::with_capacity(count);
        let mut names = NameCache::default();
        for index in 1..=count {
            let item: Table = operations.raw_get(index)?;
            let op = ui_opcode(item.raw_get::<Value>(1)?)?;
            let id = cached_name(&mut names, item.raw_get(2)?)?;
            commands.push(ui_command_for(op, id, &item, 3)?);
        }
        let mut bridge = borrow_mut(&bridge)?;
        bridge.extend_ui_commands(commands)
    })
}

/// Grouped positional protocol. One target string is decoded for all operations in a group.
/// Each operation is `{op, value...}`, for example `ui.batch_ops("status", {{"set_text", "ok"}})`.
pub(crate) fn register_ui_ops(lua: &Lua, bridge: BridgeRef) -> mlua::Result<Function> {
    lua.create_function(move |_, (id, operations): (mlua::LuaString, Table)| {
        let count = operations.raw_len();
        if count == 0 {
            return Ok(());
        }
        if count > MAX_COMMANDS_PER_FRAME {
            return Err(mlua::Error::runtime(format!(
                "ui.batch_ops contains too many commands ({count}, limit {MAX_COMMANDS_PER_FRAME})"
            )));
        }
        let mut commands = Vec::with_capacity(count);
        let id: Rc<str> = Rc::from(id.to_str()?.as_ref());
        for index in 1..=count {
            let item: Table = operations.raw_get(index)?;
            let op = ui_opcode(item.raw_get::<Value>(1)?)?;
            commands.push(ui_command_for(op, id.clone(), &item, 2)?);
        }
        let mut bridge = borrow_mut(&bridge)?;
        bridge.extend_ui_commands(commands)
    })
}

pub(crate) fn register_scene(lua: &Lua, bridge: BridgeRef) -> mlua::Result<Function> {
    lua.create_function(move |lua, operations: Table| {
        let scope = current_scene_scope(lua)?;
        let count = operations.raw_len();
        if count == 0 {
            return Ok(());
        }
        if count > MAX_COMMANDS_PER_FRAME {
            return Err(mlua::Error::runtime(format!(
                "scene.batch contains too many commands ({count}, limit {MAX_COMMANDS_PER_FRAME})"
            )));
        }
        let mut commands = Vec::with_capacity(count);
        let mut names = NameCache::default();
        for index in 1..=count {
            let item: Table = operations.raw_get(index)?;
            let op = scene_opcode(item.raw_get::<Value>("op")?)?;
            let target = ScopedNodeName {
                scope,
                name: cached_name(&mut names, item.raw_get("name")?)?,
            };
            commands.push(scene_command_keyed(op, target, &item)?);
        }
        let mut bridge = borrow_mut(&bridge)?;
        bridge.extend_scene_commands(commands)
    })
}

pub(crate) fn register_scene_compact(lua: &Lua, bridge: BridgeRef) -> mlua::Result<Function> {
    lua.create_function(move |lua, operations: Table| {
        let scope = current_scene_scope(lua)?;
        let count = operations.raw_len();
        if count == 0 {
            return Ok(());
        }
        if count > MAX_COMMANDS_PER_FRAME {
            return Err(mlua::Error::runtime(format!(
                "scene.batch_compact contains too many commands ({count}, limit {MAX_COMMANDS_PER_FRAME})"
            )));
        }
        let mut commands = Vec::with_capacity(count);
        let mut names = NameCache::default();
        for index in 1..=count {
            let item: Table = operations.raw_get(index)?;
            let op = scene_opcode(item.raw_get::<Value>(1)?)?;
            let target = ScopedNodeName {
                scope,
                name: cached_name(&mut names, item.raw_get(2)?)?,
            };
            commands.push(scene_command_for(op, target, &item, 3)?);
        }
        let mut bridge = borrow_mut(&bridge)?;
        bridge.extend_scene_commands(commands)
    })
}

pub(crate) fn register_scene_ops(lua: &Lua, bridge: BridgeRef) -> mlua::Result<Function> {
    lua.create_function(move |lua, (name, operations): (mlua::LuaString, Table)| {
        let scope = current_scene_scope(lua)?;
        let count = operations.raw_len();
        if count == 0 {
            return Ok(());
        }
        if count > MAX_COMMANDS_PER_FRAME {
            return Err(mlua::Error::runtime(format!(
                "scene.batch_ops contains too many commands ({count}, limit {MAX_COMMANDS_PER_FRAME})"
            )));
        }
        let mut commands = Vec::with_capacity(count);
        let name: Rc<str> = Rc::from(name.to_str()?.as_ref());
        for index in 1..=count {
            let item: Table = operations.raw_get(index)?;
            let op = scene_opcode(item.raw_get::<Value>(1)?)?;
            let target = ScopedNodeName {
                scope,
                name: name.clone(),
            };
            commands.push(scene_command_for(op, target, &item, 2)?);
        }
        let mut bridge = borrow_mut(&bridge)?;
        bridge.extend_scene_commands(commands)
    })
}
