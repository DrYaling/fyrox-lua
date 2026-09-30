//! Offline metadata and binding scaffold generator.
//!
//! This tool intentionally runs outside the game. It reads Rust source files,
//! emits an auditable catalog and generates only registration metadata. Runtime
//! bindings still require approval and either a generated wrapper template or a
//! handwritten implementation in lua-plugin.

mod metadata;
use quote::ToTokens;
use serde::Serialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
struct Catalog {
    schema: u32,
    source: String,
    types: Vec<TypeEntry>,
    functions: Vec<FunctionEntry>,
    methods: Vec<MethodEntry>,
    components: Vec<ComponentEntry>,
    data_types: Vec<DataTypeEntry>,
}

#[derive(Debug, Serialize)]
struct TypeEntry {
    name: String,
    kind: String,
    fields: Vec<String>,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct FunctionEntry {
    name: String,
    signature: String,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct MethodEntry {
    owner: String,
    name: String,
    signature: String,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct UnsupportedEntry {
    source: String,
    item: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize)]
struct ComponentEntry {
    name: String,
    lua_namespace: String,
    source_type: String,
    category: String,
    methods: Vec<String>,
    traits: Vec<String>,
    status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct DataTypeEntry {
    name: String,
    lua_namespace: String,
    fields: Vec<String>,
    constructors: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse(env::args().skip(1))?;
    fs::create_dir_all(&args.output)?;
    let mut files = Vec::new();
    collect_rs(&args.input, &mut files)?;
    if files.is_empty() {
        return Err(format!("no Rust source files found under {}", args.input.display()).into());
    }

    let mut catalog = Catalog {
        schema: 1,
        source: args.input.display().to_string(),
        types: Vec::new(),
        functions: Vec::new(),
        methods: Vec::new(),
        components: match args.profile.as_str() {
            "common" => common_components(),
            "none" => Vec::new(),
            profile => return Err(format!("unknown component profile: {profile}").into()),
        },
        data_types: common_data_types(),
    };
    let mut unsupported = Vec::new();
    for file in files {
        parse_file(&file, &mut catalog, &mut unsupported)?;
    }

    fs::write(
        args.output.join("curated-catalog.json"),
        serde_json::to_vec_pretty(&metadata::curated_catalog()?)?,
    )?;
    let json = serde_json::to_vec_pretty(&catalog)?;
    fs::write(args.output.join("catalog.json"), json)?;
    fs::write(
        args.output.join("unsupported.json"),
        serde_json::to_vec_pretty(&unsupported)?,
    )?;
    write_generated(&args.output.join("generated.rs"), &catalog)?;
    if let Some(runtime_output) = args.runtime_output.as_deref() {
        write_generated(runtime_output, &catalog)?;
    }
    fs::write(
        args.output.join("api-diff.md"),
        format!(
            "# Lua API Diff\n\nGenerated schema: {}\n\nTypes: {}\nFunctions: {}\nMethods: {}\nComponents: {}\nUnsupported items: {}\n",
            catalog.schema,
            catalog.types.len(),
            catalog.functions.len(),
            catalog.methods.len(),
            catalog.components.len(),
            unsupported.len()
        ),
    )?;
    Ok(())
}

struct Args {
    input: PathBuf,
    output: PathBuf,
    runtime_output: Option<PathBuf>,
    profile: String,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut input = None;
        let mut output = PathBuf::from("target/lua-bindings");
        let mut runtime_output = None;
        let mut profile = "common".to_owned();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--input" => input = args.next().map(PathBuf::from),
                "--output" => output = args.next().ok_or("--output requires a directory")?.into(),
                "--runtime-output" => {
                    runtime_output = Some(
                        args.next()
                            .ok_or("--runtime-output requires a Rust file")?
                            .into(),
                    )
                }
                "--profile" => profile = args.next().ok_or("--profile requires a name")?,
                "--help" | "-h" => {
                    println!("lua-tool --input <rust-file-or-dir> [--output <dir>]");
                    std::process::exit(0);
                }
                unknown => return Err(format!("unknown argument: {unknown}").into()),
            }
        }
        Ok(Self {
            input: input.ok_or("--input is required")?,
            output,
            runtime_output,
            profile,
        })
    }
}

fn collect_rs(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path.to_owned());
        }
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            collect_rs(&child, files)?;
        } else if child.extension().is_some_and(|ext| ext == "rs") {
            files.push(child);
        }
    }
    files.sort();
    Ok(())
}

fn parse_file(
    path: &Path,
    catalog: &mut Catalog,
    unsupported: &mut Vec<UnsupportedEntry>,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(path)?;
    let file = syn::parse_file(&source)?;
    for item in file.items {
        match item {
            syn::Item::Struct(item) if is_public(&item.vis) => catalog.types.push(TypeEntry {
                name: item.ident.to_string(),
                kind: "struct".into(),
                fields: named_fields(&item.fields),
                status: "CatalogOnly",
            }),
            syn::Item::Enum(item) if is_public(&item.vis) => catalog.types.push(TypeEntry {
                name: item.ident.to_string(),
                kind: "enum".into(),
                fields: enum_variants(&item.variants),
                status: "CatalogOnly",
            }),
            syn::Item::Fn(item) if is_public(&item.vis) => catalog.functions.push(FunctionEntry {
                name: item.sig.ident.to_string(),
                signature: quote_signature(&item.sig),
                status: "CatalogOnly",
            }),
            syn::Item::Impl(item) => {
                let owner = item.self_ty.to_token_stream().to_string();
                for method in item.items {
                    if let syn::ImplItem::Fn(method) = method {
                        if is_public(&method.vis) {
                            catalog.methods.push(MethodEntry {
                                owner: owner.clone(),
                                name: method.sig.ident.to_string(),
                                signature: quote_signature(&method.sig),
                                status: "CatalogOnly",
                            });
                        }
                    }
                }
            }
            syn::Item::Trait(item) => unsupported.push(UnsupportedEntry {
                source: path.display().to_string(),
                item: item.ident.to_string(),
                reason: "traits require a handwritten adapter or explicit generator support".into(),
            }),
            syn::Item::Union(item) => unsupported.push(UnsupportedEntry {
                source: path.display().to_string(),
                item: item.ident.to_string(),
                reason: "unions are not safe to expose automatically".into(),
            }),
            _ => {}
        }
    }
    Ok(())
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn named_fields(fields: &syn::Fields) -> Vec<String> {
    fields
        .iter()
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect()
}

fn enum_variants(
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::Token![,]>,
) -> Vec<String> {
    variants
        .iter()
        .map(|variant| variant.ident.to_string())
        .collect()
}

fn quote_signature<T: quote::ToTokens>(value: &T) -> String {
    value.to_token_stream().to_string()
}

fn common_components() -> Vec<ComponentEntry> {
    let mut components = vec![
        ComponentEntry {
            name: "UiNode".into(),
            lua_namespace: "ui.node".into(),
            source_type: "fyrox::gui::UiNode".into(),
            category: "ui".into(),
            methods: [
                "set_visible",
                "set_enabled",
                "set_width",
                "set_height",
                "set_position",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traits: vec!["Widget".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "Widget".into(),
            lua_namespace: "ui.widget".into(),
            source_type: "fyrox::gui::widget::Widget".into(),
            category: "ui".into(),
            methods: [
                "set_text",
                "append",
                "set_visible",
                "set_enabled",
                "set_width",
                "set_height",
                "set_position",
                "text",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traits: vec!["Widget".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "Text".into(),
            lua_namespace: "ui.text".into(),
            source_type: "fyrox::gui::text::Text".into(),
            category: "ui".into(),
            methods: [
                "set_text",
                "append",
                "set_visible",
                "set_enabled",
                "set_position",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traits: vec!["Widget".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "TextBox".into(),
            lua_namespace: "ui.text_box".into(),
            source_type: "fyrox::gui::text_box::TextBox".into(),
            category: "ui".into(),
            methods: [
                "set_text",
                "text",
                "set_visible",
                "set_enabled",
                "set_position",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traits: vec!["Widget".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "Button".into(),
            lua_namespace: "ui.button".into(),
            source_type: "fyrox::gui::button::Button".into(),
            category: "ui".into(),
            methods: [
                "set_text",
                "set_visible",
                "set_enabled",
                "set_position",
                "on_click",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traits: vec!["Widget".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "Node".into(),
            lua_namespace: "scene.node".into(),
            source_type: "fyrox::scene::node::Node".into(),
            category: "3d".into(),
            methods: [
                "set_position",
                "set_rotation_z",
                "set_rotation",
                "set_scale",
                "set_enabled",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traits: vec!["Base".into(), "Transform".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "Spatial".into(),
            lua_namespace: "scene.spatial".into(),
            source_type: "fyrox::scene::base::Base".into(),
            category: "3d".into(),
            methods: [
                "set_position",
                "set_rotation_z",
                "set_rotation",
                "set_scale",
                "set_enabled",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            traits: vec!["Base".into(), "Transform".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "Mesh".into(),
            lua_namespace: "scene.mesh".into(),
            source_type: "fyrox::scene::mesh::Mesh".into(),
            category: "3d".into(),
            methods: ["set_position", "set_rotation", "set_scale", "set_enabled"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            traits: vec!["Base".into(), "Transform".into()],
            status: "Implemented",
        },
        ComponentEntry {
            name: "Camera".into(),
            lua_namespace: "scene.camera".into(),
            source_type: "fyrox::scene::camera::Camera".into(),
            category: "3d".into(),
            methods: ["set_position", "set_rotation", "set_enabled"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            traits: vec!["Base".into(), "Transform".into()],
            status: "Implemented",
        },
    ];
    for (name, namespace, source_type, methods, implemented) in [
        (
            "Toggle",
            "ui.toggle",
            "fyrox::gui::check_box::CheckBox",
            vec!["set_checked"],
            true,
        ),
        (
            "Selector",
            "ui.selector",
            "fyrox::gui::dropdown_list::DropdownList",
            vec!["set_selected"],
            true,
        ),
        (
            "ScrollViewer",
            "ui.scroll_viewer",
            "fyrox::gui::scroll_viewer::ScrollViewer",
            vec!["set_scroll"],
            true,
        ),
        (
            "ScrollPanel",
            "ui.scroll_panel",
            "fyrox::gui::scroll_panel::ScrollPanel",
            vec!["set_scroll"],
            true,
        ),
        (
            "ProgressBar",
            "ui.progress_bar",
            "fyrox::gui::progress_bar::ProgressBar",
            vec!["set_progress"],
            true,
        ),
        (
            "Popup",
            "ui.popup",
            "fyrox::gui::popup::Popup",
            vec!["open", "close"],
            true,
        ),
        (
            "Input",
            "ui.input",
            "fyrox::gui::text_box::TextBox",
            vec!["text", "set_text"],
            true,
        ),
        (
            "Image",
            "ui.image",
            "fyrox::gui::image::Image",
            vec!["set_opacity"],
            true,
        ),
        (
            "Grid",
            "ui.grid",
            "fyrox::gui::grid::Grid",
            vec!["set_row", "set_column"],
            true,
        ),
        (
            "Canvas",
            "ui.canvas",
            "fyrox::gui::canvas::Canvas",
            vec!["set_position"],
            true,
        ),
        (
            "Animation",
            "scene.animation",
            "fyrox::animation::Animation",
            vec!["play", "stop", "is_playing"],
            false,
        ),
        (
            "Signal",
            "scene.signal",
            "fyrox::generic::Signal",
            vec!["connect", "emit"],
            false,
        ),
    ] {
        components.push(ComponentEntry {
            name: name.into(),
            lua_namespace: namespace.into(),
            source_type: source_type.into(),
            category: if namespace.starts_with("ui.") {
                "ui"
            } else {
                "3d"
            }
            .into(),
            methods: methods.into_iter().map(str::to_owned).collect(),
            traits: if namespace.starts_with("ui.") {
                vec!["Widget".into()]
            } else {
                Vec::new()
            },
            status: if implemented {
                "Implemented"
            } else {
                "PlannedAdapter"
            },
        });
    }
    components
}

fn common_data_types() -> Vec<DataTypeEntry> {
    [
        ("Vector2", "Vector2", vec!["x", "y"]),
        ("Vector3", "Vector3", vec!["x", "y", "z"]),
        ("Vector4", "Vector4", vec!["x", "y", "z", "w"]),
        ("Color", "Color", vec!["r", "g", "b", "a"]),
    ]
    .into_iter()
    .map(|(name, namespace, fields)| DataTypeEntry {
        name: name.into(),
        lua_namespace: namespace.into(),
        fields: fields.into_iter().map(str::to_owned).collect(),
        constructors: vec!["new".into()],
    })
    .collect()
}

fn write_generated(path: &Path, catalog: &Catalog) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut output =
        String::from("// @generated by lua-tool; executable bindings only.\nuse mlua::Lua;\n");
    output.push_str("\n/// Registers generated common value userdata constructors and fields.\npub fn register_generated_value_types(lua: &Lua) -> mlua::Result<()> {\n    crate::game_api::register_value_types(lua)\n}\n");
    output.push_str("\n/// Registers executable methods selected by the offline component profile.\npub(crate) fn register_generated_ui_methods<M: mlua::UserDataMethods<crate::game_api::UiComponentRef>>(methods: &mut M) {\n");
    let executable_ui_methods = catalog
        .components
        .iter()
        .filter(|component| component.category == "ui" && component.status == "Implemented")
        .flat_map(|component| component.methods.iter().map(String::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    for method in executable_ui_methods {
        let registration = match method {
            "set_checked" => Some("    methods.add_method(\"set_checked\", |_, this, value: bool| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetChecked(this.id.clone(), value)));\n"),
            "set_selected" => Some("    methods.add_method(\"set_selected\", |_, this, value: Option<usize>| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetSelected(this.id.clone(), value)));\n"),
            "set_scroll" => Some("    methods.add_method(\"set_scroll\", |_, this, (x, y): (f32, f32)| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetScroll(this.id.clone(), x, y)));\n"),
            "set_progress" => Some("    methods.add_method(\"set_progress\", |_, this, value: f32| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetProgress(this.id.clone(), value)));\n"),
            "open" => Some("    methods.add_method(\"open\", |_, this, ()| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetPopupOpen(this.id.clone(), true)));\n"),
            "close" => Some("    methods.add_method(\"close\", |_, this, ()| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetPopupOpen(this.id.clone(), false)));\n"),
            "set_opacity" => Some("    methods.add_method(\"set_opacity\", |_, this, value: f32| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetOpacity(this.id.clone(), value)));\n"),
            "set_row" => Some("    methods.add_method(\"set_row\", |_, this, value: usize| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetGridRow(this.id.clone(), value)));\n"),
            "set_column" => Some("    methods.add_method(\"set_column\", |_, this, value: usize| crate::game_api::queue_ui_command(this, crate::game_api::UiCommand::SetGridColumn(this.id.clone(), value)));\n"),
            _ => None,
        };
        if let Some(registration) = registration {
            output.push_str(registration);
        }
    }
    output.push_str("}\n");
    output.push_str(
        "\n/// Registers executable Lua constructors generated from the component profile.\npub fn register_generated_executable_bindings(lua: &Lua) -> mlua::Result<()> {\n",
    );
    output.push_str("    let ui: Option<mlua::Table> = lua.globals().get(\"ui\")?;\n    let scene: Option<mlua::Table> = lua.globals().get(\"scene\")?;\n");
    output.push_str("    let ui_find: Option<mlua::Function> = ui.as_ref().and_then(|table| table.get(\"find\").ok());\n    let scene_find: Option<mlua::Function> = scene.as_ref().and_then(|table| table.get(\"find\").ok());\n");
    for component in &catalog.components {
        if component.status != "Implemented" {
            continue;
        }
        let table = if component.category == "ui" {
            "ui"
        } else {
            "scene"
        };
        let finder = if component.category == "ui" {
            "ui_find"
        } else {
            "scene_find"
        };
        let short_name = component
            .lua_namespace
            .rsplit('.')
            .next()
            .unwrap_or(component.name.as_str());
        output.push_str(&format!(
            "    if let (Some(table), Some(finder)) = ({table}.as_ref(), {finder}.as_ref()) {{ let finder = finder.clone(); table.set({name:?}, lua.create_function(move |_, id: String| finder.call::<mlua::Value>(id))?)?; }}\n",
            table = table,
            finder = finder,
            name = short_name
        ));
    }
    output.push_str("    Ok(())\n}\n");
    output.push_str(
        "\n/// Backwards-compatible name for callers that only need generated component aliases.\npub fn register_generated_component_aliases(lua: &Lua) -> mlua::Result<()> {\n    register_generated_executable_bindings(lua)\n}\n",
    );
    fs::write(path, output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_public_types_methods_and_unsupported_items() {
        let path = env::temp_dir().join(format!("lua-tool-{}.rs", std::process::id()));
        fs::write(
            &path,
            "pub struct Sample { pub value: u32 } impl Sample { pub fn get(&self) -> u32 { self.value } } pub trait External {}",
        )
        .unwrap();
        let mut catalog = Catalog {
            schema: 1,
            source: path.display().to_string(),
            types: Vec::new(),
            functions: Vec::new(),
            methods: Vec::new(),
            components: Vec::new(),
            data_types: Vec::new(),
        };
        let mut unsupported = Vec::new();
        parse_file(&path, &mut catalog, &mut unsupported).unwrap();
        assert_eq!(catalog.types[0].name, "Sample");
        assert_eq!(catalog.methods[0].name, "get");
        assert_eq!(unsupported[0].item, "External");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn generated_registration_is_deterministic() {
        let output = env::temp_dir().join(format!("lua-tool-generated-{}.rs", std::process::id()));
        let catalog = Catalog {
            schema: 1,
            source: "test".into(),
            types: vec![TypeEntry {
                name: "Sample".into(),
                kind: "struct".into(),
                fields: vec!["value".into()],
                status: "CatalogOnly",
            }],
            functions: Vec::new(),
            methods: vec![MethodEntry {
                owner: "Sample".into(),
                name: "get".into(),
                signature: "fn get(&self) -> u32".into(),
                status: "CatalogOnly",
            }],
            components: common_components(),
            data_types: common_data_types(),
        };
        write_generated(&output, &catalog).unwrap();
        let generated = fs::read_to_string(&output).unwrap();
        assert!(!generated.contains("BindingRegistry"));
        assert!(!generated.contains("CatalogOnly"));
        assert!(!generated.contains("Generated catalog entry"));
        assert!(generated.contains("register_generated_component_aliases"));
        assert!(generated.contains("register_generated_executable_bindings"));
        assert!(generated.contains("lua.create_function"));
        assert!(generated.contains("table.set(\"text\""));
        let _ = fs::remove_file(output);
    }
}
