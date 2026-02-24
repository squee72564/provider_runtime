use crate::core::types::ModelCatalog;
use crate::registry::registry::ProviderRegistry;

#[test]
fn test_registry_exports_compile() {
    let default_registry = ProviderRegistry::default();
    let explicit_registry = ProviderRegistry::new(ModelCatalog::default(), None);

    let _ = default_registry;
    let _ = explicit_registry;
}
