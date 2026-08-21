#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProviderCapabilities {
    pub(crate) text_input: bool,
    pub(crate) text_output: bool,
    pub(crate) streaming: bool,
    pub(crate) reasoning: bool,
    pub(crate) tools: bool,
    pub(crate) structured_output: bool,
}

impl ProviderCapabilities {
    pub(crate) const TEXT_STREAMING: Self = Self {
        text_input: true,
        text_output: true,
        streaming: true,
        reasoning: false,
        tools: false,
        structured_output: false,
    };

    pub(crate) fn supports(&self, requested: &Self) -> bool {
        (!requested.text_input || self.text_input)
            && (!requested.text_output || self.text_output)
            && (!requested.streaming || self.streaming)
            && (!requested.reasoning || self.reasoning)
            && (!requested.tools || self.tools)
            && (!requested.structured_output || self.structured_output)
    }
}
