//! Named engine binding extensions that may be selected by a business manifest.
//!
//! These functions deliberately live in `lua-plugin`. The business TOML may
//! reference one of these stable names, but it cannot generate or replace the
//! engine implementation.

use mlua::Lua;

/// Installs the built-in UI/Scene component aliases after the host API exists.
pub fn register_component_aliases(lua: &Lua) -> mlua::Result<()> {
    crate::bindings::register_generated_component_aliases(lua)
}

/// Installs the built-in engine value constructors and userdata types.
pub fn register_value_types(lua: &Lua) -> mlua::Result<()> {
    crate::bindings::register_engine_bindings(lua)
}
