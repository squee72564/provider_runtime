use super::*;

fn model(
    provider: ProviderId,
    model_id: &str,
    display_name: Option<&str>,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    supports_tools: bool,
    supports_structured_output: bool,
) -> ModelInfo {
    ModelInfo {
        provider,
        model_id: model_id.to_string(),
        display_name: display_name.map(ToString::to_string),
        context_window,
        max_output_tokens,
        supports_tools,
        supports_structured_output,
    }
}

#[test]
fn test_static_first_merge_policy() {
    let static_catalog = ModelCatalog {
        models: vec![
            model(
                ProviderId::Openai,
                "gpt-5-mini",
                Some("Static GPT"),
                Some(128_000),
                None,
                true,
                true,
            ),
            model(
                ProviderId::Openai,
                "gpt-5-mini",
                Some("Static Duplicate"),
                Some(200_000),
                Some(10_000),
                false,
                false,
            ),
            model(
                ProviderId::Anthropic,
                "claude-3-7-sonnet",
                Some("Claude"),
                None,
                None,
                true,
                true,
            ),
        ],
    };
    let remote_catalog = ModelCatalog {
        models: vec![
            model(
                ProviderId::Openai,
                "gpt-5-mini",
                Some("Remote GPT"),
                Some(999_999),
                Some(16_000),
                false,
                false,
            ),
            model(
                ProviderId::Openrouter,
                "openrouter/auto",
                Some("Router Auto"),
                Some(1_000_000),
                Some(8_192),
                true,
                true,
            ),
            model(
                ProviderId::Openrouter,
                "openrouter/auto",
                Some("Router Duplicate"),
                Some(2_000_000),
                Some(16_384),
                false,
                false,
            ),
        ],
    };

    let merged = merge_static_and_remote_catalog(&static_catalog, &remote_catalog);
    assert_eq!(merged.models.len(), 3);

    assert_eq!(merged.models[0].provider, ProviderId::Openai);
    assert_eq!(merged.models[0].model_id, "gpt-5-mini");
    assert_eq!(merged.models[0].display_name.as_deref(), Some("Static GPT"));
    assert_eq!(merged.models[0].context_window, Some(128_000));
    assert_eq!(merged.models[0].max_output_tokens, Some(16_000));
    assert!(merged.models[0].supports_tools);
    assert!(merged.models[0].supports_structured_output);

    assert_eq!(merged.models[1].provider, ProviderId::Anthropic);
    assert_eq!(merged.models[1].model_id, "claude-3-7-sonnet");

    assert_eq!(merged.models[2].provider, ProviderId::Openrouter);
    assert_eq!(merged.models[2].model_id, "openrouter/auto");
    assert_eq!(
        merged.models[2].display_name.as_deref(),
        Some("Router Auto")
    );
    assert_eq!(merged.models[2].context_window, Some(1_000_000));
    assert_eq!(merged.models[2].max_output_tokens, Some(8_192));
    assert!(merged.models[2].supports_tools);
    assert!(merged.models[2].supports_structured_output);
}

#[test]
fn test_resolve_model_provider_deterministic() {
    let catalog = ModelCatalog {
        models: vec![
            model(
                ProviderId::Openai,
                "shared-model",
                None,
                None,
                None,
                true,
                true,
            ),
            model(
                ProviderId::Anthropic,
                "shared-model",
                None,
                None,
                None,
                true,
                true,
            ),
            model(
                ProviderId::Openrouter,
                "shared-model",
                None,
                None,
                None,
                true,
                true,
            ),
            model(
                ProviderId::Openrouter,
                "router-only",
                None,
                None,
                None,
                true,
                true,
            ),
        ],
    };

    let only =
        resolve_model_provider(&catalog, "router-only", None).expect("single provider should map");
    assert_eq!(only, ProviderId::Openrouter);

    let with_hint = resolve_model_provider(&catalog, "shared-model", Some(ProviderId::Anthropic))
        .expect("matching hint should resolve");
    assert_eq!(with_hint, ProviderId::Anthropic);

    let mismatch =
        resolve_model_provider(&catalog, "router-only", Some(ProviderId::Openai)).unwrap_err();
    assert_eq!(
        mismatch,
        RoutingError::ProviderHintMismatch {
            model: "router-only".to_string(),
            provider_hint: ProviderId::Openai,
            resolved: ProviderId::Openrouter,
        }
    );

    let ambiguous = resolve_model_provider(&catalog, "shared-model", None).unwrap_err();
    assert_eq!(
        ambiguous,
        RoutingError::AmbiguousModelRoute {
            model: "shared-model".to_string(),
            candidates: vec![
                ProviderId::Openai,
                ProviderId::Anthropic,
                ProviderId::Openrouter
            ],
        }
    );

    let not_found = resolve_model_provider(&catalog, "missing", None).unwrap_err();
    assert_eq!(
        not_found,
        RoutingError::ModelNotFound {
            model: "missing".to_string(),
        }
    );

    let case_sensitive = resolve_model_provider(&catalog, "SHARED-MODEL", None).unwrap_err();
    assert_eq!(
        case_sensitive,
        RoutingError::ModelNotFound {
            model: "SHARED-MODEL".to_string(),
        }
    );
}

#[test]
fn test_export_catalog_json_stable_output() {
    let unsorted = ModelCatalog {
        models: vec![
            model(
                ProviderId::Openrouter,
                "m2",
                Some("router"),
                None,
                None,
                true,
                true,
            ),
            model(
                ProviderId::Anthropic,
                "m1",
                Some("anthropic"),
                None,
                None,
                true,
                true,
            ),
            model(
                ProviderId::Openai,
                "m3",
                Some("openai"),
                None,
                None,
                true,
                true,
            ),
        ],
    };
    let shuffled = ModelCatalog {
        models: vec![
            model(
                ProviderId::Anthropic,
                "m1",
                Some("anthropic"),
                None,
                None,
                true,
                true,
            ),
            model(
                ProviderId::Openai,
                "m3",
                Some("openai"),
                None,
                None,
                true,
                true,
            ),
            model(
                ProviderId::Openrouter,
                "m2",
                Some("router"),
                None,
                None,
                true,
                true,
            ),
        ],
    };

    let first = export_catalog_json(&unsorted).expect("export should succeed");
    let second = export_catalog_json(&shuffled).expect("export should succeed");
    assert_eq!(first, second);

    let parsed: serde_json::Value =
        serde_json::from_str(&first).expect("export should be valid json");
    let models = parsed
        .get("models")
        .and_then(serde_json::Value::as_array)
        .expect("models should be an array");
    assert_eq!(models.len(), 3);
    assert_eq!(models[0]["provider"]["type"], "openai");
    assert_eq!(models[1]["provider"]["type"], "anthropic");
    assert_eq!(models[2]["provider"]["type"], "openrouter");
}

#[test]
fn test_builtin_static_catalog_contains_minimal_seed() {
    let catalog = builtin_static_catalog();
    assert_eq!(catalog.models.len(), 3);
    assert!(
        catalog.models.iter().any(|model| {
            model.provider == ProviderId::Openai && model.model_id == "gpt-5-mini"
        })
    );
    assert!(catalog.models.iter().any(|model| {
        model.provider == ProviderId::Anthropic && model.model_id == "claude-3-7-sonnet"
    }));
    assert!(catalog.models.iter().any(|model| {
        model.provider == ProviderId::Openrouter && model.model_id == "openrouter/auto"
    }));
}
