#![forbid(unsafe_code)]

mod client;
mod credential;
mod decode;
mod endpoint;
mod error;
mod event;
mod framing;
mod runner;

pub use credential::{CredentialScope, ProviderCredential};
pub use endpoint::{EndpointSelection, OverrideEndpoint};
pub use error::{Result, RuntimeError, RuntimeErrorKind};
pub use event::{CompletionReason, ProviderRunOutcome, ProviderStreamEvent, TokenUsage};
pub use runner::{ProviderRuntime, RuntimeLimits};
