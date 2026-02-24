use provider_runtime::core::types::{ContentPart, Message, MessageRole, ModelCatalog};
use provider_runtime::{ProviderRuntime, ProviderRuntimeBuilder};

#[test]
fn test_public_api_compiles() {
    let _builder: ProviderRuntimeBuilder = ProviderRuntime::builder();
    let runtime = ProviderRuntime::builder().build();

    let _json = runtime
        .export_catalog_json(&ModelCatalog::default())
        .expect("catalog export should serialize");
    let _json_via_module = provider_runtime::catalog::export_catalog_json(&ModelCatalog::default())
        .expect("module export should be accessible");

    let messages = vec![Message {
        role: MessageRole::Assistant,
        content: vec![ContentPart::Thinking {
            text: "reasoning".to_string(),
            provider: None,
        }],
    }];

    let _normalized = provider_runtime::handoff::normalize_handoff_messages(
        &messages,
        &provider_runtime::ProviderId::Openai,
    );

    let _runtime_path: provider_runtime::runtime::ProviderRuntime =
        ProviderRuntime::builder().build();
}
