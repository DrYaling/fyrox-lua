//! Generic scene-node lookup cache owned by the main thread.

use crate::handles::HandleToken;
use fyrox::{
    core::pool::Handle,
    graph::SceneGraph,
    scene::{node::Node, Scene},
};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct SceneRegistry {
    // Split the scope from the name so hot-path lookups can borrow `&str`
    // without constructing a temporary Rc-backed key.
    nodes: HashMap<Option<HandleToken>, HashMap<String, Handle<Node>>>,
}

impl SceneRegistry {
    #[inline]
    pub fn resolve(&mut self, scene: &Scene, name: &str) -> Option<Handle<Node>> {
        self.resolve_scoped(scene, None, name)
    }

    #[inline]
    pub fn resolve_scoped(
        &mut self,
        scene: &Scene,
        root: Option<Handle<Node>>,
        name: &str,
    ) -> Option<Handle<Node>> {
        let scope = root.map(HandleToken::from_handle);
        let scoped = self.nodes.entry(scope).or_default();
        if let Some(handle) = scoped.get(name).copied() {
            if scene.graph.try_get(handle).is_ok() {
                return Some(handle);
            }
            scoped.remove(name);
        }
        let (handle, _) = match root {
            Some(root) => scene.graph.find(root, &mut |node| node.name() == name)?,
            None => scene.graph.find_by_name_from_root(name)?,
        };
        scoped.insert(name.to_owned(), handle);
        Some(handle)
    }

    #[inline]
    pub fn token(&mut self, scene: &Scene, name: &str) -> Option<HandleToken> {
        self.resolve(scene, name).map(HandleToken::from_handle)
    }

    #[inline]
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.values().map(HashMap::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fyrox::{
        core::pool::Handle,
        graph::SceneGraph,
        scene::{base::BaseBuilder, node::Node, pivot::PivotBuilder, Scene},
    };

    #[test]
    fn scoped_lookup_is_limited_to_root_subtree_and_cached() {
        let mut scene = Scene::new();
        let left: Handle<Node> = scene
            .graph
            .add_node(PivotBuilder::new(BaseBuilder::new().with_name("Left")).build_node());
        let right: Handle<Node> = scene
            .graph
            .add_node(PivotBuilder::new(BaseBuilder::new().with_name("Right")).build_node());
        let left_child: Handle<Node> = scene
            .graph
            .add_node(PivotBuilder::new(BaseBuilder::new().with_name("Target")).build_node());
        let right_child: Handle<Node> = scene
            .graph
            .add_node(PivotBuilder::new(BaseBuilder::new().with_name("Target")).build_node());
        scene.graph.link_nodes(left_child, left);
        scene.graph.link_nodes(right_child, right);

        let mut registry = SceneRegistry::default();
        assert_eq!(
            registry.resolve_scoped(&scene, Some(left), "Target"),
            Some(left_child)
        );
        assert_eq!(
            registry.resolve_scoped(&scene, Some(right), "Target"),
            Some(right_child)
        );
        assert_eq!(registry.resolve_scoped(&scene, Some(left), "Right"), None);
        assert_eq!(registry.resolve(&scene, "Target"), Some(left_child));
        assert_eq!(registry.len(), 3);
    }
}
