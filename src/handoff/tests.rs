use super::*;
use crate::core::types::{
    ContentPart, Message, MessageRole, ToolCall, ToolResult, ToolResultContent,
};
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
fn test_handoff_normalization_is_identity_for_assistant_content() {
    let tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "lookup".to_string(),
        arguments_json: json!({"q": "rust"}),
    };
    let tool_result = ToolResult {
        tool_call_id: "call_1".to_string(),
        content: ToolResultContent::Text {
            text: "result payload".to_string(),
        },
        raw_provider_content: None,
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
        ContentPart::Text {
            text: "end".to_string(),
        },
    ])];

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Openai);
    assert_eq!(normalized, messages);
}

#[test]
fn test_handoff_normalization_preserves_non_assistant_messages() {
    let messages = vec![user(vec![ContentPart::Text {
        text: "hi".to_string(),
    }])];
    let original = messages.clone();

    let normalized = normalize_handoff_messages(&messages, &ProviderId::Anthropic);

    assert_eq!(normalized, original);
    assert_eq!(messages, original);
}

#[test]
fn test_handoff_normalization_is_idempotent() {
    let messages = vec![assistant(vec![
        ContentPart::Text {
            text: "portable".to_string(),
        },
        ContentPart::ToolCall {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "lookup".to_string(),
                arguments_json: json!({"q": "x"}),
            },
        },
    ])];

    let once = normalize_handoff_messages(&messages, &ProviderId::Openrouter);
    let twice = normalize_handoff_messages(&once, &ProviderId::Openrouter);

    assert_eq!(once, twice);
}
