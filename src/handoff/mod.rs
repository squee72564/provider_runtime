use crate::core::types::{Message, ProviderId};

pub fn normalize_handoff_messages(
    messages: &[Message],
    _target_provider: &ProviderId,
) -> Vec<Message> {
    messages.to_vec()
}

#[cfg(test)]
mod tests;
