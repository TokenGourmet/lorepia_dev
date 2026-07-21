use serde::{Deserialize, Serialize};

use crate::{LorebookCatalog, LorebookEngine, LorebookError, MAX_IMPORT_BYTES, Result};

pub const IMPORT_FORMAT: &str = "lorepia.lorebook";
pub const IMPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportTrust {
    Untrusted,
    LocallyTrusted,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportEnvelope {
    format: String,
    schema_version: u32,
    catalog: LorebookCatalog,
}

/// Imports portable data without granting activation authority. Every entry is
/// disabled; a separate local product action must enable reviewed entries.
pub fn import_catalog(bytes: &[u8]) -> Result<LorebookCatalog> {
    import_catalog_with_trust(bytes, ImportTrust::Untrusted)
}

pub fn import_catalog_with_trust(bytes: &[u8], trust: ImportTrust) -> Result<LorebookCatalog> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(LorebookError::too_large("import", MAX_IMPORT_BYTES));
    }
    let mut envelope: ImportEnvelope = serde_json::from_slice(bytes).map_err(|error| {
        if error.is_syntax() || error.is_eof() {
            LorebookError::ImportSyntax
        } else {
            LorebookError::ImportSchema
        }
    })?;
    if envelope.format != IMPORT_FORMAT {
        return Err(LorebookError::ImportSchema);
    }
    if envelope.schema_version != IMPORT_SCHEMA_VERSION {
        return Err(LorebookError::UnsupportedImportVersion);
    }
    envelope.catalog.validate()?;
    if trust == ImportTrust::Untrusted {
        for entry in envelope.catalog.entries_mut() {
            entry.set_enabled(false);
        }
    }
    Ok(envelope.catalog)
}

/// Imports a locally reviewed catalog and compiles active regex conditions
/// exactly once into the returned engine. Invalid regex is reported with its
/// entry index here; the default untrusted import intentionally performs no
/// regex compilation because every imported entry is disabled.
pub fn import_trusted_engine(bytes: &[u8]) -> Result<LorebookEngine> {
    let catalog = import_catalog_with_trust(bytes, ImportTrust::LocallyTrusted)?;
    LorebookEngine::new(catalog)
}

pub fn export_catalog(catalog: &LorebookCatalog) -> Result<Vec<u8>> {
    catalog.validate()?;
    let bytes = serde_json::to_vec_pretty(&ImportEnvelope {
        format: IMPORT_FORMAT.to_owned(),
        schema_version: IMPORT_SCHEMA_VERSION,
        catalog: catalog.clone(),
    })
    .map_err(|_| LorebookError::Serialization)?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(LorebookError::too_large("export", MAX_IMPORT_BYTES));
    }
    Ok(bytes)
}
