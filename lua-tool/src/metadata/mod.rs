mod catalog;
mod manual;
mod registry;
pub use registry::*;

/// Historical curated descriptions; not a runtime capability manifest.
pub fn curated_catalog() -> Result<BindingRegistry, String> {
    let mut registry = BindingRegistry::default();
    catalog::register(&mut registry)?;
    manual::register(&mut registry)?;
    Ok(registry)
}
