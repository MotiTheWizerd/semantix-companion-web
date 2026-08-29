use super::capabilities::ProviderCapabilities;

/// One API-key provider Companion can actually execute.
///
/// Settings, inference, and memory all read this same specification. Adding a
/// provider therefore cannot make it selectable without also giving the
/// runtime enough information to execute it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ApiProviderSpec {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) key_placeholder: &'static str,
    pub(crate) api_base_url: &'static str,
    pub(crate) chat_completions_url: &'static str,
    pub(crate) protocol: ApiProviderProtocol,
    pub(crate) capabilities: ProviderCapabilities,
    pub(crate) default_headers: &'static [(&'static str, &'static str)],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApiProviderProtocol {
    OpenAiChatCompletions,
}

const OPENAI_COMPATIBLE_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    text_input: true,
    text_output: true,
    streaming: true,
    reasoning: true,
    tools: true,
    structured_output: false,
};

pub(crate) const API_PROVIDERS: &[ApiProviderSpec] = &[
    ApiProviderSpec {
        id: "together",
        name: "Together AI",
        key_placeholder: "Enter API key",
        api_base_url: "https://api.together.ai/v1",
        chat_completions_url: "https://api.together.ai/v1/chat/completions",
        protocol: ApiProviderProtocol::OpenAiChatCompletions,
        capabilities: OPENAI_COMPATIBLE_CAPABILITIES,
        default_headers: &[],
    },
    ApiProviderSpec {
        id: "openrouter",
        name: "OpenRouter",
        key_placeholder: "sk-or-v1-…",
        api_base_url: "https://openrouter.ai/api/v1",
        chat_completions_url: "https://openrouter.ai/api/v1/chat/completions",
        protocol: ApiProviderProtocol::OpenAiChatCompletions,
        capabilities: OPENAI_COMPATIBLE_CAPABILITIES,
        // Optional attribution supported by OpenRouter. No user or workspace
        // data rides in this static application title.
        default_headers: &[("X-OpenRouter-Title", "Semantix Companion")],
    },
];

pub(crate) fn api_provider_spec(id: &str) -> Option<&'static ApiProviderSpec> {
    API_PROVIDERS.iter().find(|provider| provider.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{api_provider_spec, ApiProviderProtocol, API_PROVIDERS};

    #[test]
    fn every_selectable_api_provider_has_a_runtime_protocol_and_endpoint() {
        assert!(!API_PROVIDERS.is_empty());
        let mut ids = HashSet::new();
        for provider in API_PROVIDERS {
            assert!(!provider.id.is_empty());
            assert!(
                ids.insert(provider.id),
                "duplicate provider id: {}",
                provider.id
            );
            assert!(provider
                .chat_completions_url
                .starts_with(provider.api_base_url));
            assert_eq!(
                provider.protocol,
                ApiProviderProtocol::OpenAiChatCompletions
            );
        }
    }

    #[test]
    fn openrouter_is_connected_through_its_official_api_base() {
        let provider = api_provider_spec("openrouter").expect("OpenRouter should be connected");
        assert_eq!(provider.api_base_url, "https://openrouter.ai/api/v1");
        assert_eq!(
            provider.chat_completions_url,
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }
}
