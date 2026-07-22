#![forbid(unsafe_code)]

mod client;
mod credential;
mod decode;
mod endpoint;
mod error;
mod event;
mod framing;
mod runner;
mod token_count;

pub use credential::{CredentialScope, ProviderCredential};
pub use endpoint::{EndpointSelection, OverrideEndpoint};
pub use error::{Result, RetryDecision, RuntimeError, RuntimeErrorKind};
pub use event::{CompletionReason, ProviderRunOutcome, ProviderStreamEvent, TokenUsage};
pub use runner::{ProviderRuntime, RuntimeLimits};
