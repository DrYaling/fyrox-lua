//! Fyrox 公开 API 的手写目录。
//!
//! 这里登记的是稳定的类型边界和文档信息，而不是把 Rust 方法自动映射到 Lua。
//! 只有带有真实 wrapper 的模块才会创建 Lua 全局表；目录项用于审计、生成文档和规划后续绑定。

use super::{BindingCategory, BindingMethod, BindingRegistry, BindingStatus, BindingType};

macro_rules! catalog_types {
    ($registry:expr, $(($name:literal, $module:literal, $category:expr, $description:literal)),+ $(,)?) => {{
        $(
            $registry.register_type(BindingType {
                name: $name,
                module: $module,
                category: $category,
                description: $description,
                status: BindingStatus::CatalogOnly,
                methods: Vec::<BindingMethod>::new(),
            })?;
        )+
        Ok(())
    }};
}

/// 登记 Fyrox 公开类型。新增公开类型时必须在此处手写分类和中文说明。
pub(super) fn register(registry: &mut BindingRegistry) -> Result<(), String> {
    catalog_types!(
        registry,
        // core::math / core::color：值类型可优先实现为 Lua userdata。
        (
            "Vector2",
            "fyrox.core.math",
            BindingCategory::Core,
            "二维浮点向量。"
        ),
        (
            "Vector3",
            "fyrox.core.math",
            BindingCategory::Core,
            "三维浮点向量。"
        ),
        (
            "Vector4",
            "fyrox.core.math",
            BindingCategory::Core,
            "四维浮点向量。"
        ),
        (
            "Quaternion",
            "fyrox.core.math",
            BindingCategory::Core,
            "旋转四元数。"
        ),
        (
            "Matrix4",
            "fyrox.core.math",
            BindingCategory::Core,
            "四维变换矩阵。"
        ),
        (
            "Color",
            "fyrox.core.color",
            BindingCategory::Core,
            "线性 RGBA 颜色。"
        ),
        (
            "Rect",
            "fyrox.core.math",
            BindingCategory::Core,
            "二维矩形区域。"
        ),
        (
            "Handle",
            "fyrox.core.pool",
            BindingCategory::Core,
            "带代数的对象句柄。"
        ),
        // engine：引擎生命周期和插件接口。
        (
            "Engine",
            "fyrox.engine",
            BindingCategory::Engine,
            "Fyrox 引擎主实例。"
        ),
        (
            "Plugin",
            "fyrox.plugin",
            BindingCategory::Engine,
            "引擎插件生命周期接口。"
        ),
        (
            "PluginConstructor",
            "fyrox.plugin",
            BindingCategory::Engine,
            "插件构造器接口。"
        ),
        // scene：场景图、节点和常用组件。
        (
            "Scene",
            "fyrox.scene",
            BindingCategory::Scene,
            "场景资源和场景图容器。"
        ),
        (
            "Graph",
            "fyrox.scene.graph",
            BindingCategory::Scene,
            "场景节点图及句柄访问。"
        ),
        (
            "Node",
            "fyrox.scene.node",
            BindingCategory::Scene,
            "场景图节点基础类型。"
        ),
        (
            "Transform",
            "fyrox.scene.transform",
            BindingCategory::Scene,
            "节点位置、旋转和缩放。"
        ),
        (
            "Camera",
            "fyrox.scene.camera",
            BindingCategory::Scene3D,
            "三维摄像机组件。"
        ),
        (
            "Mesh",
            "fyrox.scene.mesh",
            BindingCategory::Scene3D,
            "网格几何组件。"
        ),
        (
            "Sprite",
            "fyrox.scene.sprite",
            BindingCategory::Scene2D,
            "二维精灵组件。"
        ),
        (
            "Rectangle",
            "fyrox.scene.rectangle",
            BindingCategory::Scene2D,
            "二维矩形组件。"
        ),
        // gui：控件类型；Button 已由真实 wrapper 注册。
        (
            "Widget",
            "fyrox.gui.widget",
            BindingCategory::Ui,
            "所有 UI 控件的基础组件。"
        ),
        (
            "Text",
            "fyrox.gui.text",
            BindingCategory::Ui,
            "文本显示控件。"
        ),
        (
            "TextBox",
            "fyrox.gui.text_box",
            BindingCategory::Ui,
            "文本输入控件。"
        ),
        (
            "CheckBox",
            "fyrox.gui.check_box",
            BindingCategory::Ui,
            "复选框控件。"
        ),
        (
            "ListView",
            "fyrox.gui.list_view",
            BindingCategory::Ui,
            "列表视图控件。"
        ),
        (
            "Window",
            "fyrox.gui.window",
            BindingCategory::Ui,
            "窗口控件。"
        ),
        (
            "Grid",
            "fyrox.gui.grid",
            BindingCategory::Ui,
            "网格布局控件。"
        ),
        (
            "StackPanel",
            "fyrox.gui.stack_panel",
            BindingCategory::Ui,
            "线性堆叠布局控件。"
        ),
        // input / resource：运行时服务，需要宿主上下文适配器。
        (
            "Input",
            "fyrox.gui.input",
            BindingCategory::Input,
            "键盘、鼠标和手柄输入状态。"
        ),
        (
            "ResourceManager",
            "fyrox.resource",
            BindingCategory::Resource,
            "资源加载和缓存服务。"
        ),
        (
            "Resource",
            "fyrox.resource",
            BindingCategory::Resource,
            "异步资源句柄基础接口。"
        ),
        (
            "Texture",
            "fyrox.resource.texture",
            BindingCategory::Rendering,
            "纹理资源。"
        ),
        (
            "Material",
            "fyrox.material",
            BindingCategory::Material,
            "材质资源及着色参数。"
        ),
        // physics / animation / audio / scripting：按模块隔离，发布版再逐项实现 wrapper。
        (
            "RigidBody",
            "fyrox.scene.rigid_body",
            BindingCategory::Physics,
            "刚体物理组件。"
        ),
        (
            "Collider",
            "fyrox.scene.collider",
            BindingCategory::Physics,
            "碰撞体组件。"
        ),
        (
            "AnimationPlayer",
            "fyrox.scene.animation",
            BindingCategory::Animation,
            "动画播放组件。"
        ),
        (
            "AudioSource",
            "fyrox.scene.sound",
            BindingCategory::Audio,
            "音频源组件。"
        ),
        (
            "Script",
            "fyrox.script",
            BindingCategory::Scripting,
            "Rust 场景脚本基础接口."
        ),
    )
}
