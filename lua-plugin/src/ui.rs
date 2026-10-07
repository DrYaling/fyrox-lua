//! Generic Fyrox UI component registry.
//!
//! This module intentionally knows no game-specific IDs. The host registers arbitrary named
//! engine nodes, while Lua resolves and operates them through userdata.
use crate::game_api::{Bridge, LuaLogLevel, SharedName, UiCommand, UiElementKind, UiElementSpec};
use crate::handles::HandleToken;
use fyrox::core::algebra::Vector2;
use fyrox::core::color::Color;
use fyrox::core::pool::Handle;
use fyrox::{
    graph::SceneGraph,
    gui::{
        brush::Brush,
        button::{Button, ButtonBuilder, ButtonContent, ButtonMessage},
        check_box::{CheckBox, CheckBoxMessage},
        dropdown_list::{DropdownList, DropdownListMessage},
        image::Image,
        popup::{Popup, PopupMessage},
        progress_bar::{ProgressBar, ProgressBarMessage},
        scroll_panel::{ScrollPanel, ScrollPanelMessage},
        scroll_viewer::{ScrollViewer, ScrollViewerMessage},
        text::{Text, TextBuilder, TextMessage},
        text_box::{TextBox, TextBoxBuilder},
        widget::{WidgetBuilder, WidgetMessage},
        UiNode, UserInterface,
    },
};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiComponent {
    Text(Handle<Text>),
    TextBox(Handle<TextBox>),
    Button(Handle<Button>),
    CheckBox(Handle<CheckBox>),
    DropdownList(Handle<DropdownList>),
    ScrollPanel(Handle<ScrollPanel>),
    ScrollViewer(Handle<ScrollViewer>),
    ProgressBar(Handle<ProgressBar>),
    Popup(Handle<Popup>),
    Image(Handle<Image>),
    Widget(Handle<UiNode>),
}

impl UiComponent {
    #[inline]
    fn handle(self) -> Handle<UiNode> {
        match self {
            Self::Text(handle) => handle.to_base(),
            Self::TextBox(handle) => handle.to_base(),
            Self::Button(handle) => handle.to_base(),
            Self::CheckBox(handle) => handle.to_base(),
            Self::DropdownList(handle) => handle.to_base(),
            Self::ScrollPanel(handle) => handle.to_base(),
            Self::ScrollViewer(handle) => handle.to_base(),
            Self::ProgressBar(handle) => handle.to_base(),
            Self::Popup(handle) => handle.to_base(),
            Self::Image(handle) => handle.to_base(),
            Self::Widget(handle) => handle,
        }
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct UiRegistry {
    components: HashMap<String, UiComponent>,
    button_names: HashMap<Handle<UiNode>, String>,
}

impl UiRegistry {
    /// Strict prefab binding for hosts that require a serialized UI contract.
    pub fn apply_checked(
        &mut self,
        ui: &mut UserInterface,
        command: UiCommand,
    ) -> Result<(), String> {
        if let UiCommand::ResolveRequired(id) = command {
            let count = ui
                .nodes()
                .pair_iter()
                .filter(|(_, node)| node.name() == id.as_ref())
                .count();
            return match count {
                1 if self.resolve(ui, id.as_ref()).is_some() => Ok(()),
                0 => Err(format!("required UI prefab node is missing: {id}")),
                1 => Err(format!(
                    "required UI prefab node could not be resolved: {id}"
                )),
                _ => Err(format!(
                    "required UI prefab node name is duplicated: {id} ({count} matches)"
                )),
            };
        }
        self.apply(ui, command);
        Ok(())
    }
    pub fn create(&mut self, ui: &mut UserInterface, spec: UiElementSpec) {
        let ctx = &mut ui.build_ctx();
        let widget = WidgetBuilder::new()
            .with_name(&spec.id)
            .with_width(spec.width)
            .with_height(spec.height)
            .with_desired_position(Vector2::new(spec.x, spec.y));
        match spec.kind {
            UiElementKind::Text => self.register_text(
                spec.id.clone(),
                TextBuilder::new(widget).with_text(spec.text).build(ctx),
            ),
            UiElementKind::TextBox => self.register_text_box(
                spec.id.clone(),
                TextBoxBuilder::new(widget).with_text(spec.text).build(ctx),
            ),
            UiElementKind::Button => self.register_button(
                spec.id.clone(),
                ButtonBuilder::new(widget).with_text(&spec.text).build(ctx),
            ),
        }
    }
    pub fn register_text(&mut self, id: impl Into<String>, handle: Handle<Text>) {
        self.components.insert(id.into(), UiComponent::Text(handle));
    }
    pub fn register_text_box(&mut self, id: impl Into<String>, handle: Handle<TextBox>) {
        self.components
            .insert(id.into(), UiComponent::TextBox(handle));
    }
    pub fn register_button(&mut self, id: impl Into<String>, handle: Handle<Button>) {
        let id = id.into();
        let base = handle.to_base();
        self.components
            .insert(id.clone(), UiComponent::Button(handle));
        self.button_names.insert(base, id);
    }
    pub fn get(&self, id: &str) -> Option<UiComponent> {
        self.components.get(id).copied()
    }

    pub fn resolve(&mut self, ui: &UserInterface, id: &str) -> Option<UiComponent> {
        if let Some(component) = self.get(id) {
            return Some(component);
        }
        let (handle, node) = ui.find_by_name_from_root(id)?;
        let component = if node.cast::<Text>().is_some() {
            UiComponent::Text(handle.to_variant())
        } else if node.cast::<TextBox>().is_some() {
            UiComponent::TextBox(handle.to_variant())
        } else if node.cast::<Button>().is_some() {
            UiComponent::Button(handle.to_variant())
        } else if node.cast::<CheckBox>().is_some() {
            UiComponent::CheckBox(handle.to_variant())
        } else if node.cast::<DropdownList>().is_some() {
            UiComponent::DropdownList(handle.to_variant())
        } else if node.cast::<ScrollPanel>().is_some() {
            UiComponent::ScrollPanel(handle.to_variant())
        } else if node.cast::<ScrollViewer>().is_some() {
            UiComponent::ScrollViewer(handle.to_variant())
        } else if node.cast::<ProgressBar>().is_some() {
            UiComponent::ProgressBar(handle.to_variant())
        } else if node.cast::<Popup>().is_some() {
            UiComponent::Popup(handle.to_variant())
        } else if node.cast::<Image>().is_some() {
            UiComponent::Image(handle.to_variant())
        } else {
            UiComponent::Widget(handle)
        };
        self.cache(id.to_owned(), component);
        Some(component)
    }

    /// Registers an engine-resolved component once. Lua userdata can then cache this handle and
    /// avoid repeating name lookup on every call.
    pub fn cache(&mut self, id: impl Into<String>, component: UiComponent) {
        let id = id.into();
        if let Some(UiComponent::Button(previous)) = self.components.insert(id.clone(), component) {
            self.button_names.remove(&previous.to_base());
        }
        if let UiComponent::Button(button) = component {
            self.button_names.insert(button.to_base(), id.clone());
        }
    }

    pub fn clear(&mut self) {
        self.components.clear();
        self.button_names.clear();
    }
    pub fn button_id(&self, destination: Handle<UiNode>) -> Option<&str> {
        self.button_names.get(&destination).map(String::as_str)
    }

    /// Returns the stable identity of an existing UI node after resolving it once.
    pub fn token(&mut self, ui: &UserInterface, id: &str) -> Option<HandleToken> {
        Some(HandleToken::from_handle(self.resolve(ui, id)?.handle()))
    }
    pub fn text_box_value(&self, ui: &UserInterface, id: &str) -> Option<String> {
        match self.get(id)? {
            UiComponent::TextBox(handle) => ui.try_get(handle).ok().map(TextBox::text),
            _ => None,
        }
    }

    pub fn read_text(&mut self, ui: &UserInterface, id: &str) -> Option<String> {
        match self.resolve(ui, id)? {
            UiComponent::Text(handle) => ui.try_get(handle).ok().map(Text::text),
            UiComponent::TextBox(handle) => ui.try_get(handle).ok().map(TextBox::text),
            _ => None,
        }
    }

    /// 将尚未建立缓存的文本从引擎同步到 bridge。
    ///
    /// 已存在的值由 Lua 命令队列维护，不能在 UI 消息尚未消费时被引擎中的旧值覆盖。
    pub fn sync_text_values(&self, ui: &UserInterface, bridge: &mut Bridge) {
        for (id, component) in &self.components {
            let value = match component {
                UiComponent::Text(handle) => ui.try_get(*handle).ok().map(Text::text),
                UiComponent::TextBox(handle) => ui.try_get(*handle).ok().map(TextBox::text),
                UiComponent::Button(_) => None,
                _ => None,
            };
            if let Some(value) = value {
                bridge.sync_ui_text_value(id, value);
            }
        }
    }

    pub fn apply(&mut self, ui: &mut UserInterface, command: UiCommand) {
        self.apply_batch(ui, std::iter::once(command));
    }

    /// Applies commands in order while reusing the last resolved target. The
    /// cache is local to this call so reloads and dynamic IDs cannot leave
    /// stale handles or retained identifiers behind.
    pub fn apply_batch(
        &mut self,
        ui: &mut UserInterface,
        commands: impl IntoIterator<Item = UiCommand>,
    ) {
        self.apply_batch_with_text_values(ui, commands, &HashMap::new());
    }

    /// 按命令顺序应用 UI 修改，并使用宿主提供的最新文本作为 append 基线。
    ///
    /// Fyrox 的 `send` 是异步消息队列。若上一帧的文本消息尚未被 UI 消费，直接从节点
    /// 读取会得到旧值；宿主缓存因此必须在本批次开始前传入，避免跨帧 append 丢失内容。
    pub fn apply_batch_with_text_values(
        &mut self,
        ui: &mut UserInterface,
        commands: impl IntoIterator<Item = UiCommand>,
        baseline: &HashMap<String, String>,
    ) {
        let mut last_target: Option<(SharedName, Option<UiComponent>)> = None;
        let mut text_values: HashMap<SharedName, String> = HashMap::new();
        for command in commands {
            self.apply_cached(ui, command, &mut last_target, &mut text_values, baseline);
        }
    }

    #[inline]
    fn resolve_cached(
        &mut self,
        ui: &UserInterface,
        id: &SharedName,
        last_target: &mut Option<(SharedName, Option<UiComponent>)>,
    ) -> Option<UiComponent> {
        if let Some((cached_id, component)) = last_target.as_ref() {
            if Rc::ptr_eq(cached_id, id) || cached_id.as_ref() == id.as_ref() {
                return *component;
            }
        }
        let component = self.resolve(ui, id.as_ref());
        *last_target = Some((id.clone(), component));
        component
    }

    pub(crate) fn apply_cached(
        &mut self,
        ui: &mut UserInterface,
        command: UiCommand,
        last_target: &mut Option<(SharedName, Option<UiComponent>)>,
        text_values: &mut HashMap<SharedName, String>,
        baseline: &HashMap<String, String>,
    ) {
        match command {
            UiCommand::Load(_) | UiCommand::Show(_) => {}
            UiCommand::Resolve(id) => {
                if self.resolve_cached(ui, &id, last_target).is_some() {
                    fyrox::core::log::Log::info(format!(
                        "[Lua] ui.find resolved existing UI node: {id}"
                    ));
                } else {
                    fyrox::core::log::Log::warn(format!(
                        "[Lua] ui.find could not resolve existing UI node: {id}"
                    ));
                }
            }
            UiCommand::ResolveRequired(id) => {
                if let Err(error) = self.apply_checked(ui, UiCommand::ResolveRequired(id)) {
                    fyrox::core::log::Log::err(format!("[Lua] {error}"));
                }
            }
            UiCommand::Create(spec) => {
                self.create(ui, spec);
                *last_target = None;
            }
            UiCommand::SetText(id, value) => match self.resolve_cached(ui, &id, last_target) {
                Some(UiComponent::Text(handle)) => {
                    text_values.insert(id, value.clone());
                    ui.send(handle, TextMessage::Text(value))
                }
                Some(UiComponent::TextBox(handle)) => {
                    text_values.insert(id, value.clone());
                    ui.send(handle, TextMessage::Text(value))
                }
                Some(UiComponent::Button(handle)) => {
                    text_values.insert(id, value.clone());
                    ui.send(handle, ButtonMessage::Content(ButtonContent::text(value)))
                }
                _ => {}
            },
            UiCommand::SetLayout(id, x, y, width, height) => {
                if let Some(component) = self.resolve_cached(ui, &id, last_target) {
                    let handle = component.handle();
                    ui.send(handle, WidgetMessage::DesiredPosition(Vector2::new(x, y)));
                    ui.send(handle, WidgetMessage::Width(width));
                    ui.send(handle, WidgetMessage::Height(height));
                }
            }
            UiCommand::SetTint(id, r, g, b, a, opacity) => {
                if let Some(component) = self.resolve_cached(ui, &id, last_target) {
                    let handle = component.handle();
                    ui.send(
                        handle,
                        WidgetMessage::Foreground(
                            Brush::Solid(Color::from_rgba(
                                (r.clamp(0.0, 1.0) * 255.0) as u8,
                                (g.clamp(0.0, 1.0) * 255.0) as u8,
                                (b.clamp(0.0, 1.0) * 255.0) as u8,
                                (a.clamp(0.0, 1.0) * 255.0) as u8,
                            ))
                            .into(),
                        ),
                    );
                    ui.send(handle, WidgetMessage::Opacity(Some(opacity)));
                }
            }
            UiCommand::Append(id, value) => match self.resolve_cached(ui, &id, last_target) {
                Some(UiComponent::Text(handle)) => {
                    let cached_text = baseline.get(id.as_ref()).cloned();
                    let current = text_values.entry(id).or_insert_with(|| {
                        cached_text.unwrap_or_else(|| {
                            ui.try_get(handle).map(Text::text).unwrap_or_default()
                        })
                    });
                    current.push_str(&value);
                    ui.send(handle, TextMessage::Text(current.clone()));
                }
                Some(UiComponent::TextBox(handle)) => {
                    let cached_text = baseline.get(id.as_ref()).cloned();
                    let current = text_values.entry(id).or_insert_with(|| {
                        cached_text.unwrap_or_else(|| {
                            ui.try_get(handle).map(TextBox::text).unwrap_or_default()
                        })
                    });
                    current.push_str(&value);
                    ui.send(handle, TextMessage::Text(current.clone()));
                }
                _ => {}
            },
            UiCommand::SetVisible(id, visible) => match self.resolve_cached(ui, &id, last_target) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Visibility(visible)),
                _ => {}
            },
            UiCommand::SetEnabled(id, enabled) => match self.resolve_cached(ui, &id, last_target) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Enabled(enabled)),
                _ => {}
            },
            UiCommand::SetWidth(id, width) => match self.resolve_cached(ui, &id, last_target) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Width(width)),
                _ => {}
            },
            UiCommand::SetHeight(id, height) => match self.resolve_cached(ui, &id, last_target) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Height(height)),
                _ => {}
            },
            UiCommand::SetPosition(id, x, y) => match self.resolve_cached(ui, &id, last_target) {
                Some(component) => ui.send(
                    component.handle(),
                    WidgetMessage::DesiredPosition(Vector2::new(x, y)),
                ),
                _ => {}
            },
            UiCommand::SetChecked(id, value) => {
                if let Some(UiComponent::CheckBox(handle)) =
                    self.resolve_cached(ui, &id, last_target)
                {
                    ui.send(handle, CheckBoxMessage::Check(Some(value)));
                }
            }
            UiCommand::SetSelected(id, value) => {
                if let Some(UiComponent::DropdownList(handle)) =
                    self.resolve_cached(ui, &id, last_target)
                {
                    ui.send(handle, DropdownListMessage::Selection(value));
                }
            }
            UiCommand::SetScroll(id, x, y) => match self.resolve_cached(ui, &id, last_target) {
                Some(UiComponent::ScrollPanel(handle)) => {
                    ui.send(handle, ScrollPanelMessage::HorizontalScroll(x));
                    ui.send(handle, ScrollPanelMessage::VerticalScroll(y));
                }
                Some(UiComponent::ScrollViewer(handle)) => {
                    ui.send(handle, ScrollViewerMessage::HorizontalScroll(x));
                    ui.send(handle, ScrollViewerMessage::VerticalScroll(y));
                }
                _ => {}
            },
            UiCommand::SetProgress(id, value) => {
                if let Some(UiComponent::ProgressBar(handle)) =
                    self.resolve_cached(ui, &id, last_target)
                {
                    ui.send(handle, ProgressBarMessage::Progress(value));
                }
            }
            UiCommand::SetPopupOpen(id, open) => {
                if let Some(UiComponent::Popup(handle)) = self.resolve_cached(ui, &id, last_target)
                {
                    ui.send(
                        handle,
                        if open {
                            PopupMessage::Open
                        } else {
                            PopupMessage::Close
                        },
                    );
                }
            }
            UiCommand::SetOpacity(id, value) => {
                if let Some(component) = self.resolve_cached(ui, &id, last_target) {
                    ui.send(component.handle(), WidgetMessage::Opacity(Some(value)));
                }
            }
            UiCommand::SetGridRow(id, value) => {
                if let Some(component) = self.resolve_cached(ui, &id, last_target) {
                    ui.send(component.handle(), WidgetMessage::Row(value));
                }
            }
            UiCommand::SetGridColumn(id, value) => {
                if let Some(component) = self.resolve_cached(ui, &id, last_target) {
                    ui.send(component.handle(), WidgetMessage::Column(value));
                }
            }
            UiCommand::SetColor(id, r, g, b, a) => {
                if let Some(component) = self.resolve_cached(ui, &id, last_target) {
                    ui.send(
                        component.handle(),
                        WidgetMessage::Foreground(
                            Brush::Solid(Color::from_rgba(
                                (r.clamp(0.0, 1.0) * 255.0) as u8,
                                (g.clamp(0.0, 1.0) * 255.0) as u8,
                                (b.clamp(0.0, 1.0) * 255.0) as u8,
                                (a.clamp(0.0, 1.0) * 255.0) as u8,
                            ))
                            .into(),
                        ),
                    );
                }
            }
            UiCommand::Log(level, message) => match level {
                LuaLogLevel::Info => fyrox::core::log::Log::info(format!("[LuaScript] {message}")),
                LuaLogLevel::Warn => fyrox::core::log::Log::warn(format!("[LuaScript] {message}")),
                LuaLogLevel::Error => fyrox::core::log::Log::err(format!("[LuaScript] {message}")),
            },
        }
    }
}

#[cfg(test)]
mod strict_binding_tests {
    use super::*;

    #[test]
    fn required_prefab_node_must_exist_once() {
        let mut ui = UserInterface::new(Vector2::new(320.0, 200.0));
        for _ in 0..2 {
            let node = TextBuilder::new(WidgetBuilder::new().with_name("duplicated"))
                .with_text("test")
                .build(&mut ui.build_ctx());
            ui.link_nodes(node.to_base::<UiNode>(), ui.root(), false);
        }
        let node = TextBuilder::new(WidgetBuilder::new().with_name("present"))
            .with_text("test")
            .build(&mut ui.build_ctx());
        ui.link_nodes(node.to_base::<UiNode>(), ui.root(), false);
        let mut registry = UiRegistry::default();
        assert!(registry
            .apply_checked(&mut ui, UiCommand::ResolveRequired("present".into()))
            .is_ok());
        assert!(registry
            .apply_checked(&mut ui, UiCommand::ResolveRequired("missing".into()))
            .unwrap_err()
            .contains("missing"));
        assert!(registry
            .apply_checked(&mut ui, UiCommand::ResolveRequired("duplicated".into()))
            .unwrap_err()
            .contains("duplicated"));
    }

    #[test]
    fn batch_text_append_uses_previous_queued_value() {
        let mut ui = UserInterface::new(Vector2::new(320.0, 200.0));
        let node = TextBuilder::new(WidgetBuilder::new().with_name("title"))
            .with_text("initial")
            .build(&mut ui.build_ctx());
        ui.link_nodes(node.to_base::<UiNode>(), ui.root(), false);
        let mut registry = UiRegistry::default();
        registry.apply_batch(
            &mut ui,
            [
                UiCommand::SetText("title".into(), "a".into()),
                UiCommand::Append("title".into(), "b".into()),
                UiCommand::SetText("title".into(), "c".into()),
                UiCommand::Append("title".into(), "d".into()),
            ],
        );
        while ui.poll_message().is_some() {}
        assert_eq!(registry.read_text(&mut ui, "title").as_deref(), Some("cd"));
    }

    #[test]
    fn cross_frame_append_uses_cached_text_before_ui_messages_are_consumed() {
        let mut ui = UserInterface::new(Vector2::new(320.0, 200.0));
        let node = TextBuilder::new(WidgetBuilder::new().with_name("title"))
            .with_text("initial")
            .build(&mut ui.build_ctx());
        ui.link_nodes(node.to_base::<UiNode>(), ui.root(), false);
        let mut registry = UiRegistry::default();
        registry.apply_batch_with_text_values(
            &mut ui,
            [UiCommand::SetText("title".into(), "a".into())],
            &HashMap::new(),
        );

        let baseline = HashMap::from([(String::from("title"), String::from("a"))]);
        registry.apply_batch_with_text_values(
            &mut ui,
            [UiCommand::Append("title".into(), "b".into())],
            &baseline,
        );
        while ui.poll_message().is_some() {}
        assert_eq!(registry.read_text(&mut ui, "title").as_deref(), Some("ab"));
    }
}
