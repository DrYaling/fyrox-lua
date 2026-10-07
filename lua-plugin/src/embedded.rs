//! Pure-Lua helpers shipped with lua-plugin.
//!
//! The source is deliberately embedded in the Rust artifact.  It is loaded
//! after engine/game registrations and before project `main.lua`, so scripts
//! can use the helpers without copying a library into `data/scripts`.
const EMBEDDED_LUA_TOOLS: &str = r#"
local fwok = rawget(_G, "fwok") or {}
local mathx = fwok.math or {}
function mathx.clamp(v, lo, hi) if v < lo then return lo elseif v > hi then return hi else return v end end
function mathx.lerp(a, b, t) return a + (b - a) * t end
function mathx.round(v) if v >= 0 then return math.floor(v + 0.5) else return math.ceil(v - 0.5) end end
function mathx.sign(v) if v < 0 then return -1 elseif v > 0 then return 1 else return 0 end end
local stringx = fwok.string or {}
function stringx.trim(v) return (v:gsub("^%s+", ""):gsub("%s+$", "")) end
function stringx.starts_with(v, prefix) return v:sub(1, #prefix) == prefix end
function stringx.ends_with(v, suffix) return suffix == "" or v:sub(-#suffix) == suffix end
local tablex = fwok.table or {}
function tablex.clear(t) for k in pairs(t) do t[k] = nil end return t end
function tablex.copy(t) local out = {}; for k, v in pairs(t) do out[k] = v end return out end
function tablex.count(t) local n = 0; for _ in pairs(t) do n = n + 1 end return n end
local list = fwok.list or {}
function list.map(t, fn) local out = {}; for i, v in ipairs(t) do out[i] = fn(v, i) end return out end
function list.filter(t, fn) local out = {}; for i, v in ipairs(t) do if fn(v, i) then out[#out + 1] = v end end return out end
function list.reduce(t, fn, initial) local acc = initial; for i, v in ipairs(t) do acc = fn(acc, v, i) end return acc end
local event_mt = {}
event_mt.__index = event_mt
local function event_error(message)
    if log and log.error then
        pcall(log.error, "fwok.event listener removed after error: " .. tostring(message))
    end
end
function event_mt:add(listener)
    assert(type(listener) == "function", "event listener must be a function")
    if self.dispatch_depth > 0 then self.pending[#self.pending + 1] = { op = "add", value = listener }
    else self.listeners[#self.listeners + 1] = listener end
    return listener
end
function event_mt:remove(listener)
    if self.dispatch_depth > 0 then
        for i = #self.listeners, 1, -1 do if self.listeners[i] == listener then self.listeners[i] = false end end
        return
    end
    for i = #self.listeners, 1, -1 do if self.listeners[i] == listener then table.remove(self.listeners, i) end end
end
function event_mt:clear()
    if self.dispatch_depth > 0 then self.pending[#self.pending + 1] = { op = "clear" }
    else for i = #self.listeners, 1, -1 do self.listeners[i] = nil end end
end
function event_mt:count() return #self.listeners end
function event_mt:emit(...)
    self.dispatch_depth = self.dispatch_depth + 1
    local failures = 0
    for i = 1, #self.listeners do
        local listener = self.listeners[i]
        if listener ~= false then
            local ok, message = xpcall(listener, tostring, ...)
            if not ok then event_error(message) end
            if not ok then self.listeners[i] = false; failures = failures + 1 end
        end
    end
    self.dispatch_depth = self.dispatch_depth - 1
    for i = #self.listeners, 1, -1 do if self.listeners[i] == false then table.remove(self.listeners, i) end end
    if self.dispatch_depth == 0 then
        for i = 1, #self.pending do
            local op = self.pending[i]
            if op.op == "add" then
                self.listeners[#self.listeners + 1] = op.value
            elseif op.op == "remove" then
                for j = #self.listeners, 1, -1 do
                    if self.listeners[j] == op.value then table.remove(self.listeners, j) end
                end
            elseif op.op == "clear" then
                for j = #self.listeners, 1, -1 do self.listeners[j] = nil end
            end
        end
        for i = #self.pending, 1, -1 do self.pending[i] = nil end
    end
    return #self.listeners, failures
end
local event = {}
function event.new() return setmetatable({ listeners = {}, pending = {}, dispatch_depth = 0 }, event_mt) end
local vec3 = {}
function vec3.new(x, y, z) return { x = x or 0, y = y or 0, z = z or 0 } end
function vec3.clone(v) return { x = v.x, y = v.y, z = v.z } end
function vec3.add(a, b) return { x = a.x + b.x, y = a.y + b.y, z = a.z + b.z } end
function vec3.sub(a, b) return { x = a.x - b.x, y = a.y - b.y, z = a.z - b.z } end
function vec3.mul(v, scalar) return { x = v.x * scalar, y = v.y * scalar, z = v.z * scalar } end
function vec3.dot(a, b) return a.x * b.x + a.y * b.y + a.z * b.z end
function vec3.sqr_magnitude(v) return v.x * v.x + v.y * v.y + v.z * v.z end
function vec3.magnitude(v) return math.sqrt(vec3.sqr_magnitude(v)) end
function vec3.distance(a, b) return vec3.magnitude(vec3.sub(a, b)) end
function vec3.lerp(a, b, t) t = mathx.clamp(t, 0, 1); return { x = a.x + (b.x - a.x) * t, y = a.y + (b.y - a.y) * t, z = a.z + (b.z - a.z) * t } end
function vec3.normalize(v)
    local length = vec3.magnitude(v)
    if length <= 1e-6 then return { x = 0, y = 0, z = 0 } end
    return { x = v.x / length, y = v.y / length, z = v.z / length }
end
fwok.math, fwok.string, fwok.table, fwok.list = mathx, stringx, tablex, list
fwok.event = event
fwok.vec3 = vec3
-- Lua-only value constructors stay on the Lua side.  This removes a Rust
-- callback and table allocation in every numeric helper call while the typed
-- Vector*/Color userdata constructors above remain available for engine APIs.
local value_fast = rawget(_G, "value_fast") or {}
function value_fast.vector2(x, y) return { x = x, y = y } end
function value_fast.vector3(x, y, z) return { x = x, y = y, z = z } end
function value_fast.vector4(x, y, z, w) return { x = x, y = y, z = z, w = w } end
function value_fast.color(r, g, b, a) return { r = r, g = g, b = b, a = a } end
fwok.value_fast = value_fast
_G.value_fast = value_fast
-- Thin aliases keep common engine access grouped under the same built-in
-- namespace without adding another Rust callback or business-specific name.
local ui_api = rawget(_G, "ui")
local scene_api = rawget(_G, "scene")
fwok.ui = fwok.ui or {}
fwok.scene = fwok.scene or {}
if ui_api then
    fwok.ui.find, fwok.ui.require, fwok.ui.batch = ui_api.find, ui_api.require, ui_api.batch
end
if scene_api then
    fwok.scene.find, fwok.scene.global_find, fwok.scene.batch = scene_api.find, scene_api.global_find, scene_api.batch
end
_G.fwok = fwok
"#;

pub(crate) fn register(lua: &mlua::Lua) -> mlua::Result<()> {
    if lua
        .globals()
        .get::<Option<bool>>("__fwok_embedded_tools")?
        .unwrap_or(false)
    {
        return Ok(());
    }
    lua.load(EMBEDDED_LUA_TOOLS)
        .set_name("@lua-plugin/embedded_tools")
        .exec()?;
    lua.globals().set("__fwok_embedded_tools", true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn helpers_are_available_without_project_files() {
        let lua = mlua::Lua::new();
        super::register(&lua).unwrap();
        let value: i32 = lua.load("return fwok.math.round(1.6)").eval().unwrap();
        assert_eq!(value, 2);
        let value: String = lua.load("return fwok.string.trim('  x ')").eval().unwrap();
        assert_eq!(value, "x");
        let value: f64 = lua
            .load("local a=fwok.vec3.new(1,2,3); local b=fwok.vec3.new(4,5,6); return fwok.vec3.dot(a,b)")
            .eval()
            .unwrap();
        assert_eq!(value, 32.0);
        let value: f64 = lua
            .load("local c=value_fast.color(0.1,0.2,0.3,1); return c.r+c.g+c.b+c.a")
            .eval()
            .unwrap();
        assert!((value - 1.6).abs() < 0.0001);
        let count: (i32, i32, i32, i32, i32, i32, i32) = lua
            .load(
                r#"
                local e = fwok.event.new()
                local seen = 0
                local f = function(v) seen = seen + v end
                e:add(f); e:add(function(v) seen = seen + v end)
                local listeners, failures = e:emit(2)
                e:remove(f)
                local after_remove = e:count()
                e:clear()
                e:add(function() error("listener failure") end)
                local after_failure, failures_seen = e:emit(1)
                return listeners, failures, seen, after_remove, e:count(), after_failure, failures_seen
                "#,
            )
            .eval::<(i32, i32, i32, i32, i32, i32, i32)>()
            .unwrap();
        assert_eq!(count, (2, 0, 4, 1, 0, 0, 1));

        let nested: (i32, i32, i32) = lua
            .load(
                r#"
                local e = fwok.event.new()
                local calls, nested_calls = 0, 0
                local f
                f = function(v)
                    calls = calls + 1
                    e:remove(f)
                    if v == 1 then
                        e:emit(2)
                    else
                        nested_calls = nested_calls + 1
                    end
                end
                e:add(f)
                e:add(function(v) if v == 2 then nested_calls = nested_calls + 1 end end)
                e:emit(1)
                return calls, nested_calls, e:count()
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(nested, (1, 1, 1));
    }
}
