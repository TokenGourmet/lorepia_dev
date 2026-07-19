use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::json::canonical_json;
use crate::schema::validate_tool_name;
use crate::{Result, ToolCall, ToolRuntimeError};

const APPROVAL_VERSION: u8 = 1;
const MAX_APPROVAL_TTL_MS: u64 = 2 * 60 * 1_000;
const MAX_CONSUMED_APPROVALS: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalToken {
    version: u8,
    nonce: u64,
    expires_at_unix_ms: u64,
    signature: String,
}

impl ApprovalToken {
    #[must_use]
    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }
}

/// Issues and verifies call-bound, short-lived, one-time approval tokens.
///
/// The host must keep one authority for the process lifetime and call
/// `issue_token` only after an explicit user approval decision. The supplied
/// key must come from a cryptographically secure process-local source.
pub struct ApprovalAuthority {
    key: Zeroizing<[u8; 32]>,
    next_nonce: AtomicU64,
    consumed: Mutex<BTreeMap<u64, u64>>,
}

impl ApprovalAuthority {
    pub fn new(key: [u8; 32]) -> Result<Self> {
        if key.iter().all(|byte| *byte == 0) {
            return Err(ToolRuntimeError::InvalidApprovalToken);
        }
        Ok(Self {
            key: Zeroizing::new(key),
            next_nonce: AtomicU64::new(1),
            consumed: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn issue_token(
        &self,
        call: &ToolCall,
        now_unix_ms: u64,
        ttl_ms: u64,
    ) -> Result<ApprovalToken> {
        call.validate()?;
        if ttl_ms == 0 || ttl_ms > MAX_APPROVAL_TTL_MS {
            return Err(ToolRuntimeError::InvalidApprovalLifetime);
        }
        let expires_at_unix_ms = now_unix_ms
            .checked_add(ttl_ms)
            .ok_or(ToolRuntimeError::InvalidApprovalLifetime)?;
        let nonce = self
            .next_nonce
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ToolRuntimeError::InternalState)?;
        let signature = self.signature(call, nonce, expires_at_unix_ms)?;
        Ok(ApprovalToken {
            version: APPROVAL_VERSION,
            nonce,
            expires_at_unix_ms,
            signature: hex_encode(&signature),
        })
    }

    fn verify_and_consume(
        &self,
        call: &ToolCall,
        token: &ApprovalToken,
        now_unix_ms: u64,
    ) -> Result<()> {
        if token.version != APPROVAL_VERSION || token.expires_at_unix_ms < now_unix_ms {
            return if token.expires_at_unix_ms < now_unix_ms {
                Err(ToolRuntimeError::ApprovalExpired)
            } else {
                Err(ToolRuntimeError::InvalidApprovalToken)
            };
        }
        if token.expires_at_unix_ms.saturating_sub(now_unix_ms) > MAX_APPROVAL_TTL_MS {
            return Err(ToolRuntimeError::InvalidApprovalToken);
        }
        let supplied =
            hex_decode_32(&token.signature).ok_or(ToolRuntimeError::InvalidApprovalToken)?;
        let verifier = self.mac(call, token.nonce, token.expires_at_unix_ms)?;
        verifier
            .verify_slice(&supplied)
            .map_err(|_| ToolRuntimeError::InvalidApprovalToken)?;

        let mut consumed = self
            .consumed
            .lock()
            .map_err(|_| ToolRuntimeError::InternalState)?;
        consumed.retain(|_, expires_at| *expires_at >= now_unix_ms);
        if consumed.contains_key(&token.nonce) {
            return Err(ToolRuntimeError::ApprovalReplay);
        }
        if consumed.len() >= MAX_CONSUMED_APPROVALS {
            return Err(ToolRuntimeError::ApprovalLedgerFull);
        }
        consumed.insert(token.nonce, token.expires_at_unix_ms);
        Ok(())
    }

    fn signature(&self, call: &ToolCall, nonce: u64, expires_at_unix_ms: u64) -> Result<[u8; 32]> {
        let mac = self.mac(call, nonce, expires_at_unix_ms)?;
        Ok(mac.finalize().into_bytes().into())
    }

    fn mac(&self, call: &ToolCall, nonce: u64, expires_at_unix_ms: u64) -> Result<Hmac<Sha256>> {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref())
            .map_err(|_| ToolRuntimeError::InternalState)?;
        mac.update(b"lorepia-tool-approval-v1");
        mac.update(&nonce.to_be_bytes());
        mac.update(&expires_at_unix_ms.to_be_bytes());
        update_field(&mut mac, call.id().as_bytes());
        update_field(&mut mac, call.name().as_bytes());
        update_field(&mut mac, &canonical_json(call.arguments())?);
        Ok(mac)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ToolPolicy {
    allowlist: BTreeSet<String>,
}

impl ToolPolicy {
    #[must_use]
    pub const fn deny_all() -> Self {
        Self {
            allowlist: BTreeSet::new(),
        }
    }

    pub fn allow_only<I, S>(tool_names: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut allowlist = BTreeSet::new();
        for tool_name in tool_names {
            let tool_name = tool_name.into();
            validate_tool_name(&tool_name)?;
            allowlist.insert(tool_name);
        }
        Ok(Self { allowlist })
    }

    #[must_use]
    pub fn is_allowlisted(&self, tool_name: &str) -> bool {
        self.allowlist.contains(tool_name)
    }

    pub(crate) fn authorize(
        &self,
        authority: &ApprovalAuthority,
        call: &ToolCall,
        token: Option<&ApprovalToken>,
        now_unix_ms: u64,
    ) -> Result<()> {
        if !self.is_allowlisted(call.name()) {
            return Err(ToolRuntimeError::PolicyDenied {
                tool_name: call.name().to_owned(),
            });
        }
        let token = token.ok_or(ToolRuntimeError::ApprovalRequired)?;
        authority.verify_and_consume(call, token, now_unix_ms)
    }
}

fn update_field(mac: &mut Hmac<Sha256>, value: &[u8]) {
    mac.update(&(value.len() as u64).to_be_bytes());
    mac.update(value);
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}

fn hex_decode_32(input: &str) -> Option<[u8; 32]> {
    if input.len() != 64 {
        return None;
    }
    let mut output = [0_u8; 32];
    for (index, pair) in input.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(output)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
