use super::*;
use crate::core::types::{ToolCall, ToolResult};
use serde_json::json;

fn assistant(content: Vec<ContentPart>) -> Message {
    Message {
        role: MessageRole::Assistant,
        content,
    }
}

fn user(content: Vec<ContentPart>) -> Message {
    Message {
        role: MessageRole::User,
        content,
    }
}

#[test]
fn test_same_provider_assistant_preserved() {
    let messages = vec![assistant(vec![
        ContentPart::Text {
            text: "before".to_string(),
        },
        ContentPart::Thinking {
            text: "reasoning".to_string(),
            provider: Some(ProviderId::Openai),
        },
        ContentPart::Text {
            text: "after".to_string(),
        },
    ])];

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Openai);

    assert_eq!(normalized, messages);
}

#[test]
fn test_cross_provider_thinking_to_tagged_text() {
    let messages = vec![assistant(vec![
        ContentPart::Text {
            text: "prefix".to_string(),
        },
        ContentPart::Thinking {
            text: "internal steps".to_string(),
            provider: Some(ProviderId::Anthropic),
        },
        ContentPart::Text {
            text: "suffix".to_string(),
        },
    ])];

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Openai);

    assert_eq!(
        normalized,
        vec![assistant(vec![
            ContentPart::Text {
                text: "prefix".to_string(),
            },
            ContentPart::Text {
                text: "<thinking>internal steps</thinking>".to_string(),
            },
            ContentPart::Text {
                text: "suffix".to_string(),
            },
        ])]
    );
}

#[test]
fn test_tool_calls_and_results_preserved() {
    let tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "lookup".to_string(),
        arguments_json: json!({"q": "rust"}),
    };
    let tool_result = ToolResult {
        tool_call_id: "call_1".to_string(),
        content: vec![ContentPart::Text {
            text: "result payload".to_string(),
        }],
    };

    let messages = vec![assistant(vec![
        ContentPart::Text {
            text: "start".to_string(),
        },
        ContentPart::ToolCall {
            tool_call: tool_call.clone(),
        },
        ContentPart::ToolResult {
            tool_result: tool_result.clone(),
        },
        ContentPart::Thinking {
            text: "cross".to_string(),
            provider: Some(ProviderId::Anthropic),
        },
        ContentPart::Text {
            text: "end".to_string(),
        },
    ])];

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Openai);

    assert_eq!(
        normalized,
        vec![assistant(vec![
            ContentPart::Text {
                text: "start".to_string(),
            },
            ContentPart::ToolCall { tool_call },
            ContentPart::ToolResult { tool_result },
            ContentPart::Text {
                text: "<thinking>cross</thinking>".to_string(),
            },
            ContentPart::Text {
                text: "end".to_string(),
            },
        ])]
    );
}

#[test]
fn test_same_api_family_openai_openrouter_preserves_thinking() {
    let messages = vec![assistant(vec![ContentPart::Thinking {
        text: "family-safe".to_string(),
        provider: Some(ProviderId::Openrouter),
    }])];

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Openai);

    assert_eq!(normalized, messages);
}

#[test]
fn test_unknown_thinking_provider_converts_to_tagged_text() {
    let messages = vec![assistant(vec![ContentPart::Thinking {
        text: "unknown-source".to_string(),
        provider: None,
    }])];

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Anthropic);

    assert_eq!(
        normalized,
        vec![assistant(vec![ContentPart::Text {
            text: "<thinking>unknown-source</thinking>".to_string(),
        }])]
    );
}

#[test]
fn test_non_assistant_messages_unchanged() {
    let messages = vec![user(vec![
        ContentPart::Text {
            text: "hi".to_string(),
        },
        ContentPart::Thinking {
            text: "user-side meta".to_string(),
            provider: None,
        },
    ])];
    let original = messages.clone();

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Openai);

    assert_eq!(normalized, original);
    assert_eq!(messages, original);
}

#[test]
fn test_normalization_is_idempotent_for_same_target() {
    let messages = vec![assistant(vec![
        ContentPart::Thinking {
            text: "portable".to_string(),
            provider: None,
        },
        ContentPart::Thinking {
            text: "family-kept".to_string(),
            provider: Some(ProviderId::Openrouter),
        },
        ContentPart::Text {
            text: "tail".to_string(),
        },
    ])];

    let once = normalize_handoff_messages(&messages, &ProviderId::Openai);
    let twice = normalize_handoff_messages(&once, &ProviderId::Openai);

    assert_eq!(once, twice);
}
