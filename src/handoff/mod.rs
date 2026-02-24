use crate::core::types::{ContentPart, Message, MessageRole, ProviderId};

pub fn normalize_handoff_messages(
    messages: &[Message],
    target_provider: &ProviderId,
) -> Vec<Message> {
    messages
        .iter()
        .map(|message| match message.role {
            MessageRole::Assistant => Message {
                role: message.role.clone(),
                content: normalize_assistant_content(&message.content, target_provider),
            },
            _ => message.clone(),
        })
        .collect()
}

fn normalize_assistant_content(
    parts: &[ContentPart],
    target_provider: &ProviderId,
) -> Vec<ContentPart> {
    parts
        .iter()
        .map(|part| match part {
            ContentPart::Thinking { text, provider } => {
                if provider
                    .as_ref()
                    .is_some_and(|source| same_api_family(source, target_provider))
                {
                    part.clone()
                } else {
                    ContentPart::Text {
                        text: format!("<thinking>{text}</thinking>"),
                    }
                }
            }
            _ => part.clone(),
        })
        .collect()
}

fn same_api_family(source: &ProviderId, target: &ProviderId) -> bool {
    provider_family(source) == provider_family(target)
}

fn provider_family(provider: &ProviderId) -> ApiFamily {
    match provider {
        ProviderId::Openai | ProviderId::Openrouter => ApiFamily::OpenAiCompatible,
        ProviderId::Anthropic => ApiFamily::Anthropic,
        ProviderId::Custom => ApiFamily::Custom,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiFamily {
    OpenAiCompatible,
    Anthropic,
    Custom,
}

#[cfg(test)]
mod tests;
