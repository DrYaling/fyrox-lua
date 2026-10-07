use mlua::Lua;
use std::{fmt, rc::Rc};

/// Registration API for the main-thread-owned Lua VM.
///
/// The plugin owns the lifecycle of the Lua VM, while implementations of this
/// trait own the API surface they expose.  Keeping the registration boundary
/// here means the plugin never needs to depend on a game's domain modules.
pub trait LuaApi: 'static {
    fn register(&self, lua: &Lua) -> mlua::Result<()>;
}

/// Registers the APIs implemented by the Lua plugin itself.  Game crates can
/// compose this with their own [`LuaApi`] implementations through
/// [`LuaApiRegistry`].
pub use crate::game_api::Api as EngineApi;

/// Backwards-compatible name for the registration trait.
pub use LuaApi as LuaGameApi;

/// A registration callback that can be supplied directly by a game crate.
pub struct LuaApiFn<F>(F);

impl<F> LuaApiFn<F> {
    pub fn new(register: F) -> Self {
        Self(register)
    }
}

impl<F> LuaApi for LuaApiFn<F>
where
    F: Fn(&Lua) -> mlua::Result<()> + 'static,
{
    fn register(&self, lua: &Lua) -> mlua::Result<()> {
        (self.0)(lua)
    }
}

/// Ordered collection of engine or game-owned Lua API registrations.
///
/// Registrations are stored behind `Rc` so a host can compose its built-in
/// engine API with the APIs supplied by a game without moving or borrowing
/// game state across the Lua runtime boundary.
#[derive(Default, Clone)]
pub struct LuaApiRegistry {
    registrations: Vec<Rc<dyn LuaApi>>,
}

impl fmt::Debug for LuaApiRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LuaApiRegistry")
            .field("registrations", &self.registrations.len())
            .finish()
    }
}

impl LuaApiRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a typed registration and preserves insertion order.
    pub fn add<A: LuaApi>(&mut self, api: A) -> &mut Self {
        self.registrations.push(Rc::new(api));
        self
    }

    /// Explicitly named alias for callers that prefer registration terminology.
    pub fn register_api<A: LuaApi>(&mut self, api: A) -> &mut Self {
        self.add(api)
    }

    /// Adds a closure, which is convenient for small game-owned bindings.
    pub fn register_fn<F>(&mut self, register: F) -> &mut Self
    where
        F: Fn(&Lua) -> mlua::Result<()> + 'static,
    {
        self.add(LuaApiFn::new(register))
    }

    /// Appends registrations from another registry without moving it.
    pub fn extend(&mut self, other: &Self) -> &mut Self {
        self.registrations
            .extend(other.registrations.iter().cloned());
        self
    }

    pub fn len(&self) -> usize {
        self.registrations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }
}

impl LuaApi for LuaApiRegistry {
    fn register(&self, lua: &Lua) -> mlua::Result<()> {
        for registration in &self.registrations {
            registration.register(lua)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_runs_callbacks_in_registration_order() {
        let mut registry = LuaApiRegistry::new();
        registry
            .register_fn(|lua| lua.globals().set("registration_order", "engine"))
            .register_fn(|lua| {
                let current: String = lua.globals().get("registration_order")?;
                lua.globals()
                    .set("registration_order", format!("{current},game"))
            });

        let lua = Lua::new();
        LuaApi::register(&registry, &lua).unwrap();
        assert_eq!(
            lua.globals().get::<String>("registration_order").unwrap(),
            "engine,game"
        );
    }
}
