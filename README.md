# FWOK Lua Runtime and Tools

## Scope

This repository contains the standalone Lua runtime plugin and offline Lua tooling used by FWOK. It does not contain game-specific gameplay code. The workspace is resolved from the sibling `Fyrox` checkout when used inside FWOK.

本仓库包含 FWOK 使用的独立 Lua 运行时插件和离线 Lua 工具，不包含具体游戏逻辑。在 FWOK 主工程中使用时，Fyrox 引擎依赖解析到相邻的 `Fyrox` 源码目录。

## Covered Features / 功能范围

- `lua-plugin`: single-threaded `mlua` runtime integrated with Fyrox.
- `main.lua`: optional project entry point with `on_awake`, `start`, `update(dt)`, and `on_destroy` lifecycle callbacks.
- `LuaComponent`: scene-owned script resource and instance lifecycle. Only instances created by `LuaComponent` are registered in `main.scripts`.
- Configurable module search root from `data/fyrox-lua.toml`; missing configuration falls back to `data/scripts`.
- Standard Lua `require(name)` and project-facing `import(name)` alias. Both load modules on demand through `package.path` and `package.loaded`.
- Typed UI and scene bridge commands, scoped scene lookup, event dispatch, lifecycle error context, and bounded per-frame command buffers.
- `lua-tool`: offline metadata inspection and binding generation support.
- Optional benchmark APIs behind the `benchmark` feature.

- `lua-plugin`：与 Fyrox 集成的单线程 `mlua` 运行时。
- `main.lua`：可选项目入口，支持 `on_awake`、`start`、`update(dt)`、`on_destroy` 生命周期。
- `LuaComponent`：场景脚本资源和实例生命周期管理。只有由 `LuaComponent` 创建的实例才会登记到 `main.scripts`。
- Lua 模块搜索根由 `data/fyrox-lua.toml` 配置；配置文件缺失时默认使用 `data/scripts`。
- 支持标准 Lua `require(name)` 和项目约定的 `import(name)` 别名。二者都通过 `package.path` 和 `package.loaded` 按需加载模块。
- 类型化 UI/场景桥接命令、场景作用域查找、事件分发、生命周期错误上下文和有界的每帧命令缓冲。
- `lua-tool`：离线元数据检查和绑定生成工具。
- benchmark API 仅在启用 `benchmark` feature 时编译。

## Runtime Design / 运行时设计

1. The game reads `data/fyrox-lua.toml` and constructs `LuaConfig`. `script_root` is the canonical Lua source and module search root.
2. `LuaRuntime` creates one Lua VM, registers engine bindings, sets `package.path`, and aliases `import` to `require`.
3. If `<script_root>/main.lua` exists, it is evaluated and its returned table is instantiated. Its `on_awake` runs during initialization.
4. The scene graph is scanned for `LuaComponent` values. The selected Lua resource is loaded, `new(class, params)` creates an instance, and `on_awake` runs.
5. `start`, per-frame `update`, events, and destruction are dispatched by the runtime. `main.lua` and component instances share one VM and module cache.
6. Lua bindings append typed commands to a bridge. Fyrox applies those commands on the game thread; Lua does not retain engine borrows.

1. 游戏读取 `data/fyrox-lua.toml` 并生成 `LuaConfig`。`script_root` 是 Lua 源码和模块的唯一搜索根。
2. `LuaRuntime` 创建一个 Lua VM，注册引擎绑定，设置 `package.path`，并将 `import` 指向 `require`。
3. 如果 `<script_root>/main.lua` 存在，运行时执行并实例化它返回的表，然后调用 `on_awake`。
4. 运行时扫描场景中的 `LuaComponent`。加载其 Lua 资源，通过 `new(class, params)` 创建实例并调用 `on_awake`。
5. 运行时统一分发 `start`、每帧 `update`、事件和销毁回调。`main.lua` 与组件实例共享同一个 VM 和模块缓存。
6. Lua 绑定只向桥接缓冲追加类型化命令，由 Fyrox 在游戏线程执行；Lua 不持有引擎对象借用。

## Script Rules / 脚本规则

Files under `script_root` are searchable modules, not an automatic startup list. A normal Lua file executes only when imported or required code references it. `main.scripts` contains component instances keyed by the runtime instance ID; imported module tables do not appear there.

脚本目录是可搜索的模块目录，不是自动启动清单。普通 Lua 文件只有被 `import` 或 `require` 引用时才执行。`main.scripts` 保存运行时实例 ID 对应的组件实例，不保存模块表。

## Verification / 验证

Run from this directory / 在本目录执行：

```powershell
rtk cargo test --manifest-path Cargo.toml --target-dir target -p lua-plugin --lib
rtk cargo check --manifest-path Cargo.toml
```
