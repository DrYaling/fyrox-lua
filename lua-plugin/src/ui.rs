//! Generic Fyrox UI component registry.
//!
//! This module intentionally knows no game-specific IDs. The host registers arbitrary named
//! engine nodes, while Lua resolves and operates them through userdata.
use crate::game_api::{LuaLogLevel, UiCommand, UiElementKind, UiElementSpec};
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
            self.button_names.insert(button.to_base(), id);
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

    pub fn sync_text_values(&self, ui: &UserInterface, values: &mut HashMap<String, String>) {
        for (id, component) in &self.components {
            let value = match component {
                UiComponent::Text(handle) => ui.try_get(*handle).ok().map(Text::text),
                UiComponent::TextBox(handle) => ui.try_get(*handle).ok().map(TextBox::text),
                UiComponent::Button(_) => None,
                _ => None,
            };
            if let Some(value) = value {
                values.insert(id.clone(), value);
            }
        }
    }

    pub fn apply(&mut self, ui: &mut UserInterface, command: UiCommand) {
        match command {
            UiCommand::Load(_) | UiCommand::Show(_) => {}
            UiCommand::Resolve(id) => {
                if self.resolve(ui, &id).is_some() {
                    fyrox::core::log::Log::info(format!(
                        "[Lua] ui.find resolved existing UI node: {id}"
                    ));
                } else {
                    fyrox::core::log::Log::warn(format!(
                        "[Lua] ui.find could not resolve existing UI node: {id}"
                    ));
                }
            }
            UiCommand::Create(spec) => self.create(ui, spec),
            UiCommand::SetText(id, value) => match self.resolve(ui, &id) {
                Some(UiComponent::Text(handle)) => ui.send(handle, TextMessage::Text(value)),
                Some(UiComponent::TextBox(handle)) => ui.send(handle, TextMessage::Text(value)),
                Some(UiComponent::Button(handle)) => {
                    ui.send(handle, ButtonMessage::Content(ButtonContent::text(value)))
                }
                _ => {}
            },
            UiCommand::Append(id, value) => match self.resolve(ui, &id) {
                Some(UiComponent::Text(handle)) => {
                    let current = ui.try_get(handle).map(Text::text).unwrap_or_default();
                    ui.send(handle, TextMessage::Text(format!("{current}{value}")));
                }
                Some(UiComponent::TextBox(handle)) => {
                    let current = ui.try_get(handle).map(TextBox::text).unwrap_or_default();
                    ui.send(handle, TextMessage::Text(format!("{current}{value}")));
                }
                _ => {}
            },
            UiCommand::SetVisible(id, visible) => match self.resolve(ui, &id) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Visibility(visible)),
                _ => {}
            },
            UiCommand::SetEnabled(id, enabled) => match self.resolve(ui, &id) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Enabled(enabled)),
                _ => {}
            },
            UiCommand::SetWidth(id, width) => match self.resolve(ui, &id) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Width(width)),
                _ => {}
            },
            UiCommand::SetHeight(id, height) => match self.resolve(ui, &id) {
                Some(component) => ui.send(component.handle(), WidgetMessage::Height(height)),
                _ => {}
            },
            UiCommand::SetPosition(id, x, y) => match self.resolve(ui, &id) {
                Some(component) => ui.send(
                    component.handle(),
                    WidgetMessage::DesiredPosition(Vector2::new(x, y)),
                ),
                _ => {}
            },
            UiCommand::SetChecked(id, value) => {
                if let Some(UiComponent::CheckBox(handle)) = self.resolve(ui, &id) {
                    ui.send(handle, CheckBoxMessage::Check(Some(value)));
                }
            }
            UiCommand::SetSelected(id, value) => {
                if let Some(UiComponent::DropdownList(handle)) = self.resolve(ui, &id) {
                    ui.send(handle, DropdownListMessage::Selection(value));
                }
            }
            UiCommand::SetScroll(id, x, y) => match self.resolve(ui, &id) {
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
                if let Some(UiComponent::ProgressBar(handle)) = self.resolve(ui, &id) {
                    ui.send(handle, ProgressBarMessage::Progress(value));
                }
            }
            UiCommand::SetPopupOpen(id, open) => {
                if let Some(UiComponent::Popup(handle)) = self.resolve(ui, &id) {
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
                if let Some(component) = self.resolve(ui, &id) {
                    ui.send(component.handle(), WidgetMessage::Opacity(Some(value)));
                }
            }
            UiCommand::SetGridRow(id, value) => {
                if let Some(component) = self.resolve(ui, &id) {
                    ui.send(component.handle(), WidgetMessage::Row(value));
                }
            }
            UiCommand::SetGridColumn(id, value) => {
                if let Some(component) = self.resolve(ui, &id) {
                    ui.send(component.handle(), WidgetMessage::Column(value));
                }
            }
            UiCommand::SetColor(id, r, g, b, a) => {
                if let Some(component) = self.resolve(ui, &id) {
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
