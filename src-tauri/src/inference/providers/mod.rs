mod claude;
mod openai_compatible;
mod test;

pub(crate) use claude::{set_bundled_sidecar_dir, ClaudeProvider};
pub(crate) use openai_compatible::OpenAiCompatibleProvider;
pub(crate) use test::TestProvider;
