# FWOK Lua Runtime and Tools

## Scope

This repository contains the standalone Lua runtime plugin and offline Lua tooling used by FWOK. It does not contain game-specific gameplay code. The workspace resolves Fyrox from `https://github.com/DrYaling/Fyrox.git` on the `master` branch.

本仓库包含 FWOK 使用的独立 Lua 运行时插件和离线 Lua 工具，不包含具体游戏逻辑。工作区从 `https://github.com/DrYaling/Fyrox.git` 的 `master` 分支解析 Fyrox 引擎依赖。

## Covered Features / 功能范围

- `lua-plugin`: single-threaded `mlua` runtime integrated with Fyrox.
- `main.lua`: optional project entry point with `on_awake`, `start`, `update(dt)`, and `on_destroy` lifecycle callbacks.
- `LuaComponent`: scene-owned script resource and instance lifecycle. Only instances created by `LuaComponent` are registered in `main.scripts`.
- Configurable module search root from `data/fyrox-lua.toml`; missing configuration falls back to `data/scripts`.
- Standard Lua `require(name)` and project-facing `import(name)` alias. Both load modules on demand through `package.path` and `package.loaded`.
- Typed UI and scene bridge commands, scoped scene lookup, event dispatch, lifecycle error context, and bounded per-frame command buffers.
- `lua-tool`: offline metadata inspection and binding generation support.
- Optional benchmark APIs behind the `benchmark` feature.
- Runtime-owned source loading is bounded by `max_script_bytes` (default 4 MiB),
  preventing an accidental or untrusted script from forcing an unbounded parse
  allocation. This is a Rust-side guard; project `require` remains controlled by
  the configured script root and deployment policy.
- Built-in `fwok` helpers are embedded in `lua-plugin`; they are loaded into the
  VM without a project file. `fwok.math`, `fwok.string`, `fwok.table`,
  `fwok.list`, `fwok.vec3`, and a mutation-safe `fwok.event` are pure Lua.
- `ui.batch` and `scene.batch` accept readable operation tables. The positional
  `ui.batch_compact` and `scene.batch_compact` forms avoid repeated keyed fields;
  `ui.batch_ops(id, ops)` and `scene.batch_ops(name, ops)` also send the target
  name only once for operations on one object. Parsing is raw, atomic, and
  subject to the same 20,000-command frame limit.
- High-frequency proxies expose `set_layout(x, y, width, height)`,
  `set_tint(r, g, b, a, opacity)`, and scene `set_transform(position, rotation,
  scale)` to submit related operations in one protected call and one queued
  command. The host resolves each target once. Proxy and scene command
  identifiers use shared immutable names, so repeated setters do not allocate
  a new target string.
- `Vector2.new`/`Vector3.new`/`Vector4.new`/`Color.new` keep their userdata
  compatibility contract. For numeric Lua-side loops, `value_fast.vector2`,
  `value_fast.vector3`, `value_fast.vector4`, and `value_fast.color` return
  plain Lua tables through pure embedded Lua functions, without a Rust callback.
  Their field reads stay inside Lua and avoid userdata field callbacks. Tables
  are accepted by typed engine arguments as well.

Batch example:

```lua
ui.batch_ops("hud_title", {
    { "set_text", "Connecting..." },
    { "set_visible", true },
    { "set_enabled", false },
})

scene.batch_ops("Player", {
    { "set_position", 0, 1, 0 },
    { "set_rotation", 0, 0, 1.57 },
})

ui.batch_compact({
    { "set_progress", "loading_bar", 0.75 },
    { "set_opacity", "loading_icon", 0.5 },
})
```

UI operation names are `set_text`, `set_visible`, `set_enabled`,
`set_progress`, `set_opacity`, `set_position`, `set_layout`, and `set_tint`.
Scene operations are `set_position`, `set_rotation`, `set_scale`,
`set_transform`, and `set_enabled`. In compact batches each operation starts
with its name, then
the target name when using `batch_compact`, followed by values in the order
shown by the corresponding proxy method. In `batch_ops`, the target is passed
once as the first function argument, then each operation contains only its
name and values.

Hot UI properties are readable and writable. The setter updates the Lua-side
read value synchronously; the typed command is applied by the `lua-plugin` host
in its UI command phase and then enters Fyrox's normal UI message pipeline:

```lua
local txt = ui.text("hud_title")
txt.text = "qwe"
local t = txt.text
assert(t == "qwe")
txt.position = Vector2.new(10, 20)
txt.color = Color.new(1, 0.8, 0.2, 1)
txt.opacity = 0.75
txt.visible = true
txt.enabled = true
txt.width = 240
txt.height = 48
assert(txt.opacity == 0.75 and txt.width == 240 and txt.height == 48)
```

The same read-after-write behavior applies to `text`, `position`, `color`,
`opacity`, `visible`, `enabled`, `width`, and `height`. After the host accepts
text commands, its text read cache follows the UI registry so later TextBox or
engine-side text changes do not remain hidden by an old optimistic value.
Within one host command phase, consecutive setters for the same target reuse
one resolved component/node while preserving command order. The cache is
discarded after the phase; UI reload and scene changes therefore cannot retain
stale engine handles.

Numeric setters reject NaN and infinity before queue commit. Lua receives a
recoverable error and the invalid command batch remains atomic.

The old `set_text`, `set_position`, and `set_color` methods remain available.
Use `get_text()` when method syntax is preferred. Compact protocols accept
numeric opcodes as well as names: UI `1..8` are text, visibility, enabled,
progress, opacity, position, layout, tint; Scene `1..5` are position, rotation,
scale, enabled, transform.

The release benchmark uses seven samples and reports the median. It compares
direct setters, keyed batches, positional compact batches, and grouped batches
with identical queued UI operations. It also compares the old three-command
layout reference to the one-command composite layout path. See
`docs/lua-engineering-audit.md` for measured results and limitations; the 1:2
Rust/Lua target is not inferred from unrelated arithmetic or rendering timings.
The current release measurements are recorded in
`docs/lua-engineering-audit.md`.
- UI and scene proxy caches use weak values, and runtime teardown/reload removes
  scope-owned click callbacks. This follows tolua's translator/delegate lifetime
  model and prevents stale Lua closures from retaining old component instances.

`lua-tool` is deliberately business agnostic. It only scans Rust AST, validates
the declarative binding manifest, writes catalog metadata, and generates the
configured registration adapter. Engine bindings are compiled into
`lua-plugin`; the tool cannot generate them from a business target. A target
may provide `api.context`, ordered `registrations`, and optional
`custom_bindings` that point to `lua_plugin::custom_bindings::*`.

The repository uses:

```powershell
rtk cargo run --manifest-path lua/Cargo.toml --bin lua-tool -- --config data/editor/lua/lua-bindings.toml
```

The implementation is split into `config.rs` (TOML schema), `model.rs`
(catalog types), `generator_scan.rs` (Rust AST scanning), `generator_api.rs`
(configured registration adapter generation), `generator_io.rs` (atomic output
and path handling), `generator.rs` (orchestration), and `audit.rs` (generic
serialized UI contract audit). Product names and required resource IDs belong
in configuration files, never in the tool source.

### Binding manifest example / 绑定配置示例

```toml
schema = 1

[[targets]]
name = "game"
inputs = ["../game/src", "../shared-api/src"]
output = "../target/lua-bindings/game"
api_output = "../game/src/lua_bindings_generated.rs"
profile = "none"

[targets.api]
type_name = "ProjectLuaApiContext"

[[targets.api.context]]
name = "state"
type = "crate::State"

[[targets.api.registrations]]
path = "crate::register_state"
args = ["state"]

[[targets.api.custom_bindings]]
path = "lua_plugin::custom_bindings::register_value_types"
```

`registrations` only wires explicitly implemented Rust adapters; scanning a
public struct does not expose it to Lua automatically. `custom_bindings` is
restricted to named engine adapters implemented in `lua-plugin`.
`runtime_output` and non-`none` profiles are rejected, so the manifest cannot
replace engine bindings. The generated adapter is the only registration call
needed in Game:

```rust
lua_bindings_generated::register_project_apis(&mut host, context)?;
```

When Lua calls `ui.load`, `LuaPluginHost::update_with_context` returns the
resource paths after applying generic UI/Scene commands. The game submits those
paths to Fyrox's loader and calls `complete_ui_load`; it does not implement the
command queues or registries itself.

- `lua-plugin`：与 Fyrox 集成的单线程 `mlua` 运行时。
- `main.lua`：可选项目入口，支持 `on_awake`、`start`、`update(dt)`、`on_destroy` 生命周期。
- `LuaComponent`：场景脚本资源和实例生命周期管理。只有由 `LuaComponent` 创建的实例才会登记到 `main.scripts`。
- Lua 模块搜索根由 `data/fyrox-lua.toml` 配置；配置文件缺失时默认使用 `data/scripts`。
- 支持标准 Lua `require(name)` 和项目约定的 `import(name)` 别名。二者都通过 `package.path` 和 `package.loaded` 按需加载模块。
- 类型化 UI/场景桥接命令、场景作用域查找、事件分发、生命周期错误上下文和有界的每帧命令缓冲。
- `lua-tool`：离线元数据检查和绑定生成工具。
- benchmark API 仅在启用 `benchmark` feature 时编译。
- 运行时加载脚本受 `max_script_bytes` 限制（默认 4 MiB），避免异常或不可信脚本在解析前制造
  无界内存分配。这是 Rust 侧边界；项目 `require` 仍由配置的脚本根目录和发布策略控制。
- 内置 `fwok` 工具以内嵌字符串存在于 `lua-plugin`，启动时直接注入 VM，不需要项目文件。
  `fwok.math`、`fwok.string`、`fwok.table`、`fwok.list`、`fwok.vec3` 和可安全修改监听器的
  `fwok.event` 都是纯 Lua 实现。
- `ui.batch`、`scene.batch` 接收操作数组，只跨 Rust 边界一次。大批量操作使用它们；极小批量
  直接调用 userdata setter 更快，因为批量表解析有固定成本。
- UI/Scene 代理缓存使用弱值，运行时销毁和脚本热重载会移除作用域内 click callback，参考 tolua
  translator/delegate 的生命周期管理，避免旧闭包保留旧组件实例。

## Runtime Design / 运行时设计

1. The game reads `data/fyrox-lua.toml` and constructs `LuaConfig`. `script_root` is the canonical Lua source and module search root.
2. `LuaRuntime` creates one Lua VM, registers engine bindings, and installs a host-backed `import` loader under the configured script root.
3. If `<script_root>/main.lua` exists, it is evaluated and its returned table is instantiated. Its `on_awake` runs during initialization.
4. The scene graph is scanned for `LuaComponent` values. The selected Lua resource is loaded, `new(class, params)` creates an instance, and `on_awake` runs.
5. `start`, per-frame `update`, events, and destruction are dispatched by the runtime. `main.lua` and component instances share one VM and module cache.
6. Lua bindings append typed commands to a bridge. `lua-plugin` owns the command host, UI/Scene registries, queue limits, and command application on the game thread; the game only supplies its scene handle and adapts UI resource loading. Lua does not retain engine borrows.
7. Every lifecycle, update, event, and UI callback is invoked through mlua's protected call path. A failed callback unwinds its Lua stack, is logged with its phase and script path, and is isolated from the remaining callbacks. A script that fails during update or an event is quarantined for that phase so it cannot flood logs or block later frames. Rust hosts can inspect structured diagnostics with LuaPluginHost::take_errors().
8. Runtime hot paths avoid cloning the update index list, sample event logs instead of logging every event, and use `VecDeque` for bounded diagnostics. `on_click` has an explicit `off_click`; reload removes callbacks belonging to the old scene scope and teardown clears the callback table.

1. 游戏读取 `data/fyrox-lua.toml` 并生成 `LuaConfig`。`script_root` 是 Lua 源码和模块的唯一搜索根。
2. `LuaRuntime` 创建一个 Lua VM，注册引擎绑定，并在配置的脚本根目录下安装由宿主提供源码的 `import` loader。
3. 如果 `<script_root>/main.lua` 存在，运行时执行并实例化它返回的表，然后调用 `on_awake`。
4. 运行时扫描场景中的 `LuaComponent`。加载其 Lua 资源，通过 `new(class, params)` 创建实例并调用 `on_awake`。
5. 运行时统一分发 `start`、每帧 `update`、事件和销毁回调。`main.lua` 与组件实例共享同一个 VM 和模块缓存。
6. Lua 绑定只向桥接缓冲追加类型化命令。`lua-plugin` 负责命令宿主、UI/Scene 注册表、队列上限和游戏线程应用；Game 只提供场景句柄并适配项目 UI 资源加载，Lua 不持有引擎对象借用。
7. 所有生命周期、更新、事件和 UI 回调都通过 mlua 的受保护调用路径执行。回调失败后 Lua 栈会被展开恢复，运行时记录阶段和脚本路径，并继续执行其它回调。update 或事件失败的脚本会被隔离，避免每帧刷错或阻塞后续逻辑。Rust 宿主可通过 LuaPluginHost::take_errors() 读取结构化诊断。
8. 运行时热路径不再复制 update 索引列表，事件日志改为采样输出，有界诊断使用 `VecDeque`。`on_click` 提供显式 `off_click`；热重载会移除旧场景作用域 callback，销毁时清空 callback 表。

## Script Rules / 脚本规则

Files under `script_root` are searchable modules, not an automatic startup list. A normal Lua file executes only when imported or required code references it. `main.scripts` contains component instances keyed by the runtime instance ID; imported module tables do not appear there.

脚本目录是可搜索的模块目录，不是自动启动清单。普通 Lua 文件只有被 `import` 或 `require` 引用时才执行。`main.scripts` 保存运行时实例 ID 对应的组件实例，不保存模块表。

## Verification / 验证

Run from this directory / 在本目录执行：

```powershell
rtk cargo test --manifest-path Cargo.toml --target-dir target -p lua-plugin --lib
rtk cargo check --manifest-path Cargo.toml
```
