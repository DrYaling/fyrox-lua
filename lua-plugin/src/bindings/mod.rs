//! Executable Lua bindings only. Offline descriptions live in lua-tool.
mod generated;
use mlua::Lua;

/// Register value types; host APIs must be installed before component aliases.
pub fn register_engine_bindings(lua: &Lua) -> mlua::Result<()> {
    generated::register_generated_value_types(lua)
}

pub fn register_generated_component_aliases(lua: &Lua) -> mlua::Result<()> {
    generated::register_generated_executable_bindings(lua)
}
pub(crate) use generated::register_generated_ui_methods;
