//! Handwritten bindings for high-frequency and engine-specific operations.
//!
//! The actual userdata implementations live in `game_api`/`ui`; this module
//! records their stable public surface in the audit registry.

use super::{BindingCategory, BindingMethod, BindingRegistry, BindingStatus, BindingType};

pub fn register(registry: &mut BindingRegistry) -> Result<(), String> {
    registry.register_type(BindingType {
        name: "UiNodeRef",
        module: "fyrox.gui",
        category: BindingCategory::Ui,
        description: "Cached UI node proxy backed by a real UserInterface handle.",
        status: BindingStatus::Implemented,
        methods: vec![
            BindingMethod {
                name: "set_text",
                signature: "(string) -> nil",
                description: "Sets text on a cached Text, TextBox, or Button node.",
            },
            BindingMethod {
                name: "set_visible",
                signature: "(boolean) -> nil",
                description: "Sends a visibility message to a cached UI node.",
            },
            BindingMethod {
                name: "set_enabled",
                signature: "(boolean) -> nil",
                description: "Enables or disables an existing UI node.",
            },
            BindingMethod {
                name: "set_position",
                signature: "(number, number) -> nil",
                description: "Changes an existing UI widget desired position.",
            },
            BindingMethod {
                name: "on_click",
                signature: "(function) -> nil",
                description: "Registers a Lua callback for a real Button click.",
            },
        ],
    })?;
    registry.register_type(BindingType {
        name: "SceneNodeRef",
        module: "fyrox.scene",
        category: BindingCategory::Scene,
        description: "Cached scene node proxy backed by a generation-checked handle.",
        status: BindingStatus::Implemented,
        methods: vec![
            BindingMethod {
                name: "set_position",
                signature: "(number, number, number) -> nil",
                description: "Updates a cached scene node transform.",
            },
            BindingMethod {
                name: "set_rotation_z",
                signature: "(number) -> nil",
                description: "Updates a cached scene node rotation.",
            },
            BindingMethod {
                name: "set_rotation",
                signature: "(number, number, number) -> nil",
                description: "Updates roll, pitch, and yaw on an existing scene node.",
            },
            BindingMethod {
                name: "set_scale",
                signature: "(number, number, number) -> nil",
                description: "Updates scale on an existing scene node.",
            },
            BindingMethod {
                name: "set_enabled",
                signature: "(boolean) -> nil",
                description: "Enables or disables an existing scene node.",
            },
        ],
    })?;
    registry.register_type(BindingType {
        name: "HandleToken",
        module: "fyrox.core.pool",
        category: BindingCategory::Core,
        description: "Serializable index/generation identity for a main-thread handle.",
        status: BindingStatus::Implemented,
        methods: Vec::new(),
    })?;
    for binding in [
        BindingType {
            name: "UiWidgetRef",
            module: "fyrox.gui.widget",
            category: BindingCategory::Ui,
            description: "通用 UI Widget 代理，解析已有 UserInterface 节点。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_visible",
                    signature: "(boolean) -> nil",
                    description: "设置已有控件可见性。",
                },
                BindingMethod {
                    name: "set_enabled",
                    signature: "(boolean) -> nil",
                    description: "设置已有控件交互状态。",
                },
                BindingMethod {
                    name: "set_position",
                    signature: "(number, number) -> nil",
                    description: "设置已有控件期望位置。",
                },
                BindingMethod {
                    name: "set_width",
                    signature: "(number) -> nil",
                    description: "设置已有控件宽度。",
                },
                BindingMethod {
                    name: "set_height",
                    signature: "(number) -> nil",
                    description: "设置已有控件高度。",
                },
            ],
        },
        BindingType {
            name: "UiTextRef",
            module: "fyrox.gui.text",
            category: BindingCategory::Ui,
            description: "已有 Text 节点的文本代理。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_text",
                    signature: "(string) -> nil",
                    description: "设置文本内容。",
                },
                BindingMethod {
                    name: "append",
                    signature: "(string) -> nil",
                    description: "追加文本内容。",
                },
                BindingMethod {
                    name: "text",
                    signature: "() -> string",
                    description: "读取已同步文本。",
                },
            ],
        },
        BindingType {
            name: "UiTextBoxRef",
            module: "fyrox.gui.text_box",
            category: BindingCategory::Ui,
            description: "已有 TextBox 节点的输入代理。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_text",
                    signature: "(string) -> nil",
                    description: "设置输入框内容。",
                },
                BindingMethod {
                    name: "text",
                    signature: "() -> string",
                    description: "读取已同步输入内容。",
                },
                BindingMethod {
                    name: "set_enabled",
                    signature: "(boolean) -> nil",
                    description: "设置输入框是否可编辑。",
                },
            ],
        },
        BindingType {
            name: "UiButtonRef",
            module: "fyrox.gui.button",
            category: BindingCategory::Ui,
            description: "已有 Button 节点的按钮代理。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_text",
                    signature: "(string) -> nil",
                    description: "设置按钮文本内容。",
                },
                BindingMethod {
                    name: "on_click",
                    signature: "(function) -> nil",
                    description: "监听真实 ButtonMessage::Click。",
                },
                BindingMethod {
                    name: "set_enabled",
                    signature: "(boolean) -> nil",
                    description: "设置按钮是否可交互。",
                },
            ],
        },
        BindingType {
            name: "SceneNode3DRef",
            module: "fyrox.scene.node",
            category: BindingCategory::Scene3D,
            description: "已有 3D 场景节点的 generation 校验代理。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_position",
                    signature: "(number, number, number) -> nil",
                    description: "设置已有节点位置。",
                },
                BindingMethod {
                    name: "set_rotation",
                    signature: "(number, number, number) -> nil",
                    description: "设置 roll/pitch/yaw。",
                },
                BindingMethod {
                    name: "set_rotation_z",
                    signature: "(number) -> nil",
                    description: "设置 Z 轴旋转。",
                },
                BindingMethod {
                    name: "set_scale",
                    signature: "(number, number, number) -> nil",
                    description: "设置已有节点缩放。",
                },
                BindingMethod {
                    name: "set_enabled",
                    signature: "(boolean) -> nil",
                    description: "启用或禁用已有节点。",
                },
            ],
        },
        BindingType {
            name: "SceneSpatialRef",
            module: "fyrox.scene.base",
            category: BindingCategory::Scene3D,
            description: "3D Spatial/Transform 能力的通用节点代理。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_position",
                    signature: "(number, number, number) -> nil",
                    description: "设置位置。",
                },
                BindingMethod {
                    name: "set_rotation",
                    signature: "(number, number, number) -> nil",
                    description: "设置三轴旋转。",
                },
                BindingMethod {
                    name: "set_scale",
                    signature: "(number, number, number) -> nil",
                    description: "设置缩放。",
                },
            ],
        },
        BindingType {
            name: "SceneMeshRef",
            module: "fyrox.scene.mesh",
            category: BindingCategory::Scene3D,
            description: "已有 Mesh 节点的通用变换代理。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_position",
                    signature: "(number, number, number) -> nil",
                    description: "设置网格节点位置。",
                },
                BindingMethod {
                    name: "set_rotation",
                    signature: "(number, number, number) -> nil",
                    description: "设置网格节点旋转。",
                },
                BindingMethod {
                    name: "set_scale",
                    signature: "(number, number, number) -> nil",
                    description: "设置网格节点缩放。",
                },
            ],
        },
        BindingType {
            name: "SceneCameraRef",
            module: "fyrox.scene.camera",
            category: BindingCategory::Scene3D,
            description: "已有 Camera 节点的通用变换与启用代理。",
            status: BindingStatus::Implemented,
            methods: vec![
                BindingMethod {
                    name: "set_position",
                    signature: "(number, number, number) -> nil",
                    description: "设置摄像机位置。",
                },
                BindingMethod {
                    name: "set_rotation",
                    signature: "(number, number, number) -> nil",
                    description: "设置摄像机旋转。",
                },
                BindingMethod {
                    name: "set_enabled",
                    signature: "(boolean) -> nil",
                    description: "启用或禁用摄像机节点。",
                },
            ],
        },
    ] {
        registry.register_type(binding)?;
    }
    Ok(())
}
