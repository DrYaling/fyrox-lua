use mlua::Lua;

/// Registration API for the main-thread-owned Lua VM.
pub trait LuaGameApi: 'static {
    fn register(&self, lua: &Lua) -> mlua::Result<()>;
}
