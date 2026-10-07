//! Generic Fyrox host-side application of Lua bridge commands.
//!
//! This module contains no project or gameplay knowledge. It owns the queued
//! UI/scene command state and applies commands to already serialized resources.

use crate::{BridgeHandle, SceneCommand, SceneRegistry, ScopedNodeName, UiCommand, UiRegistry};
use fyrox::{
    core::{algebra::Vector3, instant::Instant, pool::Handle, warn},
    graph::SceneGraph,
    gui::{font::FontResource, UserInterface},
    plugin::PluginContext,
    scene::{node::Node, Scene},
};

#[inline]
fn resolve_scene_target(
    registry: &mut SceneRegistry,
    scene: &Scene,
    target: &ScopedNodeName,
    last_target: &mut Option<(ScopedNodeName, Option<Handle<Node>>)>,
) -> Option<Handle<Node>> {
    if let Some((cached, handle)) = last_target.as_ref() {
        if cached == target {
            return *handle;
        }
    }
    let handle = registry.resolve_scoped(
        scene,
        target.scope.map(|token| token.to_handle()),
        &target.name,
    );
    *last_target = Some((target.clone(), handle));
    handle
}

const MAX_PENDING_UI_COMMANDS: usize = 20_000;

#[derive(Debug)]
pub struct LuaCommandHost {
    scene: Handle<Scene>,
    ui_handle: Handle<UserInterface>,
    ui_registry: Option<UiRegistry>,
    scene_registry: Option<SceneRegistry>,
    bridge: BridgeHandle,
    pending_ui_commands: Vec<UiCommand>,
    pending_ui_work: usize,
    ui_visible: bool,
    ui_loading: bool,
    load_requests: Vec<String>,
}

impl LuaCommandHost {
    pub fn new(scene: Handle<Scene>) -> Self {
        Self {
            scene,
            ui_handle: Handle::NONE,
            ui_registry: None,
            scene_registry: Some(SceneRegistry::default()),
            bridge: BridgeHandle::default(),
            pending_ui_commands: Vec::new(),
            pending_ui_work: 0,
            ui_visible: true,
            ui_loading: false,
            load_requests: Vec::new(),
        }
    }

    pub fn set_scene(&mut self, scene: Handle<Scene>) {
        self.scene = scene;
        if let Some(registry) = self.scene_registry.as_mut() {
            registry.clear();
        }
    }
    pub fn scene(&self) -> Handle<Scene> {
        self.scene
    }
    pub fn ui_handle(&self) -> Handle<UserInterface> {
        self.ui_handle
    }
    pub fn bridge(&self) -> &BridgeHandle {
        &self.bridge
    }
    pub fn bridge_mut(&mut self) -> &mut BridgeHandle {
        &mut self.bridge
    }
    pub fn ui_registry(&self) -> Option<&UiRegistry> {
        self.ui_registry.as_ref()
    }
    pub fn ui_visible(&self) -> bool {
        self.ui_visible
    }
    pub fn take_ui_load_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.load_requests)
    }

    pub fn complete_ui_load(
        &mut self,
        mut ui: UserInterface,
        context: &mut PluginContext,
        font: Option<&FontResource>,
    ) {
        self.ui_loading = false;
        if let Some(font) = font {
            ui.default_font = font.clone();
        }
        if self.ui_handle.is_some() {
            context.user_interfaces.remove(self.ui_handle);
        }
        self.ui_handle = context.user_interfaces.add(ui);
        self.ui_registry = Some(UiRegistry::default());
        if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
            bridge.ui_text.clear();
            bridge.ui_text_pending.clear();
            bridge.rebuild_ui_state(self.pending_ui_commands.iter());
        }
    }

    pub fn fail_ui_load(&mut self) {
        self.ui_loading = false;
    }

    pub fn install_bridge(&mut self, bridge: BridgeHandle) {
        self.bridge = bridge;
    }

    pub fn update(&mut self, context: &mut PluginContext) {
        self.apply_ui_commands(context);
        self.apply_scene_commands(context);
    }

    pub fn apply_ui_commands(&mut self, context: &mut PluginContext) {
        let started = Instant::now();
        let commands = self
            .bridge
            .0
            .try_borrow_mut()
            .ok()
            .map(|mut bridge| bridge.take_ui_commands())
            .unwrap_or_default();
        if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
            bridge.stats.queued_commands += commands.len() as u64;
        }
        let mut ordered = std::mem::take(&mut self.pending_ui_commands);
        self.pending_ui_work = 0;
        ordered.extend(commands);
        let mut deferred = Vec::new();
        for command in ordered {
            match command {
                UiCommand::Load(path) => {
                    if self.ui_loading {
                        warn!("[Lua] ui.load ignored while loading: {}", path);
                        if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                            bridge.stats.dropped_commands += 1;
                        }
                        continue;
                    }
                    self.ui_loading = true;
                    self.load_requests.push(path);
                }
                UiCommand::Show(visible) => {
                    self.ui_visible = visible;
                    if let Ok(ui) = context.user_interfaces.try_get_mut(self.ui_handle) {
                        ui.send(
                            ui.root(),
                            fyrox::gui::widget::WidgetMessage::Visibility(visible),
                        );
                    }
                }
                command => deferred.push(command),
            }
        }
        if self.ui_loading {
            let mut dropped = 0usize;
            for command in deferred {
                let cost = command.work_cost();
                if self.pending_ui_work + cost <= MAX_PENDING_UI_COMMANDS {
                    self.pending_ui_work += cost;
                    self.pending_ui_commands.push(command);
                } else {
                    dropped += 1;
                }
            }
            if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                bridge.stats.dropped_commands += dropped as u64;
                bridge.stats.apply_duration_ns += started.elapsed().as_nanos() as u64;
            }
        } else if let (Some(registry), Ok(ui)) = (
            self.ui_registry.as_mut(),
            context.user_interfaces.try_get_mut(self.ui_handle),
        ) {
            let applied = deferred.len() as u64;
            let mut baseline = std::collections::HashMap::new();
            if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                let bridge = &mut *bridge;
                registry.sync_text_values(ui, bridge);
                // 只复制本批次 append/set_text 涉及的目标，避免每帧复制整个文本缓存。
                for command in &deferred {
                    let id = match command {
                        UiCommand::SetText(id, _) | UiCommand::Append(id, _) => id,
                        _ => continue,
                    };
                    if let Some(value) = bridge.ui_text.get(id.as_ref()) {
                        baseline
                            .entry(id.as_ref().to_owned())
                            .or_insert_with(|| value.clone());
                    }
                }
            }
            if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                bridge.commit_ui_text_commands(&deferred);
            }
            registry.apply_batch_with_text_values(ui, deferred, &baseline);
            if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                bridge.stats.applied_commands += applied;
                bridge.stats.apply_duration_ns += started.elapsed().as_nanos() as u64;
            }
        } else {
            let mut dropped = 0usize;
            for command in deferred {
                let cost = command.work_cost();
                if self.pending_ui_work + cost <= MAX_PENDING_UI_COMMANDS {
                    self.pending_ui_work += cost;
                    self.pending_ui_commands.push(command);
                } else {
                    dropped += 1;
                }
            }
            if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                bridge.stats.dropped_commands += dropped as u64;
                bridge.stats.apply_duration_ns += started.elapsed().as_nanos() as u64;
            }
        }
    }

    pub fn apply_scene_commands(&mut self, context: &mut PluginContext) {
        let started = Instant::now();
        let commands = self
            .bridge
            .0
            .try_borrow_mut()
            .ok()
            .map(|mut bridge| bridge.take_scene_commands());
        let Some(commands) = commands else { return };
        let count = commands.len() as u64;
        if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
            bridge.stats.queued_commands += count;
        }
        let Ok(scene) = context.scenes.try_get_mut(self.scene) else {
            if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                bridge.stats.dropped_commands += count;
                bridge.stats.apply_duration_ns += started.elapsed().as_nanos() as u64;
            }
            return;
        };
        let Some(registry) = self.scene_registry.as_mut() else {
            if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
                bridge.stats.dropped_commands += count;
                bridge.stats.apply_duration_ns += started.elapsed().as_nanos() as u64;
            }
            return;
        };
        let mut last_scene_target = None;
        for command in commands {
            match command {
                SceneCommand::Resolve(target) => {
                    let _ = resolve_scene_target(registry, scene, &target, &mut last_scene_target);
                }
                SceneCommand::SetTransform(target, position, rotation, scale) => {
                    if let Some(handle) =
                        resolve_scene_target(registry, scene, &target, &mut last_scene_target)
                    {
                        if let Ok(node) = scene.graph.try_get_mut(handle) {
                            node.local_transform_mut()
                                .set_position(Vector3::new(position.x, position.y, position.z));
                            node.set_rotation_angles(rotation.x, rotation.y, rotation.z);
                            node.set_scale_xyz(scale.x, scale.y, scale.z);
                        }
                    }
                }
                SceneCommand::SetPosition(target, x, y, z) => {
                    if let Some(handle) =
                        resolve_scene_target(registry, scene, &target, &mut last_scene_target)
                    {
                        if let Ok(node) = scene.graph.try_get_mut(handle) {
                            node.local_transform_mut()
                                .set_position(Vector3::new(x, y, z));
                        }
                    }
                }
                SceneCommand::SetRotationZ(target, angle) => {
                    if let Some(handle) =
                        resolve_scene_target(registry, scene, &target, &mut last_scene_target)
                    {
                        if let Ok(node) = scene.graph.try_get_mut(handle) {
                            node.set_rotation_z(angle);
                        }
                    }
                }
                SceneCommand::SetRotationAngles(target, roll, pitch, yaw) => {
                    if let Some(handle) =
                        resolve_scene_target(registry, scene, &target, &mut last_scene_target)
                    {
                        if let Ok(node) = scene.graph.try_get_mut(handle) {
                            node.set_rotation_angles(roll, pitch, yaw);
                        }
                    }
                }
                SceneCommand::SetScale(target, x, y, z) => {
                    if let Some(handle) =
                        resolve_scene_target(registry, scene, &target, &mut last_scene_target)
                    {
                        if let Ok(node) = scene.graph.try_get_mut(handle) {
                            node.set_scale_xyz(x, y, z);
                        }
                    }
                }
                SceneCommand::SetEnabled(target, enabled) => {
                    if let Some(handle) =
                        resolve_scene_target(registry, scene, &target, &mut last_scene_target)
                    {
                        if let Ok(node) = scene.graph.try_get_mut(handle) {
                            node.set_enabled(enabled);
                        }
                    }
                }
            }
        }
        if let Ok(mut bridge) = self.bridge.0.try_borrow_mut() {
            bridge.stats.applied_commands += count;
            bridge.stats.apply_duration_ns += started.elapsed().as_nanos() as u64;
        }
    }
}
