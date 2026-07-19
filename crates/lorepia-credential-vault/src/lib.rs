#![forbid(unsafe_code)]

mod backend;
mod error;
mod secret;
mod vault;

pub use error::{CredentialVaultError, CredentialVaultErrorCode, Result};
pub use secret::{MAX_SECRET_BYTES, SecretBytes, SecretInput};
pub use vault::{CredentialStatus, CredentialVault};
