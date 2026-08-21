use async_trait::async_trait;
use zeroize::Zeroizing;

use super::{capabilities::ProviderCapabilities, protocol::InferenceDelta, InferenceRequest};
use crate::streaming::{DeltaSink, StreamError};

pub(crate) enum ProviderCredential {
    None,
    ApiKey(Zeroizing<String>),
}

impl ProviderCredential {
    pub(crate) fn api_key(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::ApiKey(value) => Some(value.as_str()),
        }
    }
}

#[async_trait]
pub(crate) trait InferenceProvider: Send + Sync {
    fn id(&self) -> &'static str;

    fn capabilities(&self) -> ProviderCapabilities;

    async fn stream(
        &self,
        request: &InferenceRequest,
        credential: &ProviderCredential,
        sink: &dyn DeltaSink<InferenceDelta>,
    ) -> Result<(), StreamError>;
}
