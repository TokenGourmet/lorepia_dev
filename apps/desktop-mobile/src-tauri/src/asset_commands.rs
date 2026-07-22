use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use lorepia_assets::{
    AssetError, AssetLimits, AssetStats, AssetStore, CURRENT_ASSET_SCHEMA_VERSION,
};
use serde::Serialize;
use tauri::State;

const ASSET_DIRECTORY_NAME: &str = "assets";
const ASSET_STATUS_CONTRACT_VERSION: u8 = 1;
const MAX_ASSET_STATUS_RESPONSE_BYTES: usize = 4_096;

const MAX_ASSET_OBJECT_BYTES: u64 = 1024 * 1024 * 1024;
// The catalog stores byte totals as signed SQLite INTEGER values. Disk free space remains the
// practical capacity limit; this guard only prevents arithmetic from crossing that native bound.
const MAX_TOTAL_ASSET_BYTES: u64 = i64::MAX as u64;
const MAX_ASSET_IMAGE_WIDTH: u32 = 16_384;
const MAX_ASSET_IMAGE_HEIGHT: u32 = 16_384;
const MAX_ASSET_IMAGE_PIXELS: u64 = 67_108_864;

#[derive(Clone)]
pub(crate) struct AssetStoreState {
    asset_root: Option<PathBuf>,
    backend: Arc<OnceLock<AssetBackend>>,
}

enum AssetBackend {
    Ready(AssetStore),
    Unavailable(AssetStatusErrorCode),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetStatusErrorCode {
    PathUnavailable,
    SchemaIncompatible,
    FilesystemUnsafe,
    StoreUnavailable,
    Internal,
}

impl AssetStatusErrorCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PathUnavailable => "ASSET_PATH_UNAVAILABLE",
            Self::SchemaIncompatible => "ASSET_SCHEMA_INCOMPATIBLE",
            Self::FilesystemUnsafe => "ASSET_FILESYSTEM_UNSAFE",
            Self::StoreUnavailable => "ASSET_STORE_UNAVAILABLE",
            Self::Internal => "ASSET_INTERNAL",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssetStoreStatusResponse {
    contract_version: u8,
    available: bool,
    supported_schema_version: i64,
    error_code: Option<&'static str>,
    limits: AssetLimitsResponse,
    stats: Option<AssetStatsResponse>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetLimitsResponse {
    max_object_bytes: String,
    max_total_bytes: String,
    max_image_width: u32,
    max_image_height: u32,
    max_image_pixels: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetStatsResponse {
    object_count: String,
    active_bytes: String,
    reference_count: String,
    missing_count: String,
    quarantined_count: String,
    staging_count: String,
}

impl AssetStoreState {
    pub(crate) fn new(app_local_data_dir: Result<PathBuf, tauri::Error>) -> Self {
        Self::from_asset_root(
            app_local_data_dir
                .ok()
                .map(|directory| directory.join(ASSET_DIRECTORY_NAME)),
        )
    }

    fn from_asset_root(asset_root: Option<PathBuf>) -> Self {
        Self {
            asset_root,
            backend: Arc::new(OnceLock::new()),
        }
    }

    async fn status(&self) -> AssetStoreStatusResponse {
        let state = self.clone();
        let response =
            match tauri::async_runtime::spawn_blocking(move || state.status_blocking()).await {
                Ok(response) => response,
                Err(_) => AssetStoreStatusResponse::unavailable(AssetStatusErrorCode::Internal),
            };
        enforce_ipc_ceiling(response)
    }

    fn status_blocking(&self) -> AssetStoreStatusResponse {
        let backend = self.backend.get_or_init(|| self.open_backend());
        match backend {
            AssetBackend::Ready(store) => match store.stats() {
                Ok(stats) => AssetStoreStatusResponse::available(stats),
                Err(error) => AssetStoreStatusResponse::unavailable(map_asset_error(&error)),
            },
            AssetBackend::Unavailable(code) => AssetStoreStatusResponse::unavailable(*code),
        }
    }

    fn open_backend(&self) -> AssetBackend {
        let Some(asset_root) = self.asset_root.as_ref() else {
            return AssetBackend::Unavailable(AssetStatusErrorCode::PathUnavailable);
        };
        let limits = match product_asset_limits() {
            Ok(limits) => limits,
            Err(_) => return AssetBackend::Unavailable(AssetStatusErrorCode::Internal),
        };
        match AssetStore::open(asset_root, limits) {
            Ok(store) => AssetBackend::Ready(store),
            Err(error) => AssetBackend::Unavailable(map_asset_error(&error)),
        }
    }
}

impl AssetStoreStatusResponse {
    fn available(stats: AssetStats) -> Self {
        Self {
            contract_version: ASSET_STATUS_CONTRACT_VERSION,
            available: true,
            supported_schema_version: CURRENT_ASSET_SCHEMA_VERSION,
            error_code: None,
            limits: AssetLimitsResponse::product(),
            stats: Some(stats.into()),
        }
    }

    fn unavailable(code: AssetStatusErrorCode) -> Self {
        Self {
            contract_version: ASSET_STATUS_CONTRACT_VERSION,
            available: false,
            supported_schema_version: CURRENT_ASSET_SCHEMA_VERSION,
            error_code: Some(code.as_str()),
            limits: AssetLimitsResponse::product(),
            stats: None,
        }
    }
}

impl AssetLimitsResponse {
    fn product() -> Self {
        Self {
            max_object_bytes: MAX_ASSET_OBJECT_BYTES.to_string(),
            max_total_bytes: MAX_TOTAL_ASSET_BYTES.to_string(),
            max_image_width: MAX_ASSET_IMAGE_WIDTH,
            max_image_height: MAX_ASSET_IMAGE_HEIGHT,
            max_image_pixels: MAX_ASSET_IMAGE_PIXELS,
        }
    }
}

impl From<AssetStats> for AssetStatsResponse {
    fn from(stats: AssetStats) -> Self {
        Self {
            object_count: stats.object_count.to_string(),
            active_bytes: stats.active_bytes.to_string(),
            reference_count: stats.reference_count.to_string(),
            missing_count: stats.missing_count.to_string(),
            quarantined_count: stats.quarantined_count.to_string(),
            staging_count: stats.staging_count.to_string(),
        }
    }
}

fn product_asset_limits() -> lorepia_assets::Result<AssetLimits> {
    AssetLimits::new(MAX_ASSET_OBJECT_BYTES, MAX_TOTAL_ASSET_BYTES)?.with_image_limits(
        MAX_ASSET_IMAGE_WIDTH,
        MAX_ASSET_IMAGE_HEIGHT,
        MAX_ASSET_IMAGE_PIXELS,
    )
}

fn map_asset_error(error: &AssetError) -> AssetStatusErrorCode {
    match error {
        AssetError::SchemaVersion { .. }
        | AssetError::IncompatibleCatalog { .. }
        | AssetError::HashMetadataConflict { .. } => AssetStatusErrorCode::SchemaIncompatible,
        AssetError::UnsafeFilesystem { .. } => AssetStatusErrorCode::FilesystemUnsafe,
        AssetError::Io(_) | AssetError::Database(_) => AssetStatusErrorCode::StoreUnavailable,
        AssetError::InvalidInput { .. }
        | AssetError::NotFound { .. }
        | AssetError::NotActive { .. }
        | AssetError::MimeMismatch { .. }
        | AssetError::UnsupportedContent
        | AssetError::LimitExceeded { .. }
        | AssetError::Cancelled
        | AssetError::LockPoisoned
        | AssetError::MutationBusy
        | AssetError::SnapshotCancelled => AssetStatusErrorCode::Internal,
    }
}

fn enforce_ipc_ceiling(response: AssetStoreStatusResponse) -> AssetStoreStatusResponse {
    if serde_json::to_vec(&response)
        .is_ok_and(|bytes| bytes.len() <= MAX_ASSET_STATUS_RESPONSE_BYTES)
    {
        response
    } else {
        AssetStoreStatusResponse::unavailable(AssetStatusErrorCode::Internal)
    }
}

#[tauri::command]
pub(crate) async fn get_asset_store_status(
    assets: State<'_, AssetStoreState>,
) -> Result<AssetStoreStatusResponse, &'static str> {
    let assets = assets.inner().clone();
    Ok(assets.status().await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn state_is_lazy_and_first_status_opens_only_the_native_asset_root() {
        let directory = tempdir().expect("temporary app data");
        let app_data = directory.path().join("app-data");
        let asset_root = app_data.join(ASSET_DIRECTORY_NAME);
        let state = AssetStoreState::from_asset_root(Some(asset_root.clone()));

        assert!(state.backend.get().is_none());
        assert!(!app_data.exists());

        let response = state.status().await;

        assert!(response.available);
        assert!(response.error_code.is_none());
        assert!(state.backend.get().is_some());
        assert!(asset_root.join("assets.sqlite3").is_file());
        assert!(!app_data.join("lorepia.sqlite3").exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_has_fixed_limits_decimal_stats_and_a_closed_ipc_ceiling() {
        let directory = tempdir().expect("temporary asset root");
        let state = AssetStoreState::from_asset_root(Some(directory.path().join("assets")));
        let response = state.status().await;
        let value = serde_json::to_value(&response).expect("status JSON");
        let bytes = serde_json::to_vec(&response).expect("status bytes");

        assert_eq!(value["contractVersion"], 1);
        assert_eq!(value["supportedSchemaVersion"], 4);
        assert_eq!(value["limits"]["maxObjectBytes"], "1073741824");
        assert_eq!(value["limits"]["maxTotalBytes"], i64::MAX.to_string());
        assert_eq!(value["limits"]["maxImageWidth"], 16_384);
        assert_eq!(value["limits"]["maxImageHeight"], 16_384);
        assert_eq!(value["limits"]["maxImagePixels"], 67_108_864);
        for field in [
            "objectCount",
            "activeBytes",
            "referenceCount",
            "missingCount",
            "quarantinedCount",
            "stagingCount",
        ] {
            assert_eq!(value["stats"][field], "0");
        }
        assert!(bytes.len() <= MAX_ASSET_STATUS_RESPONSE_BYTES);
    }

    #[test]
    fn maximum_decimal_stats_still_fit_the_ipc_ceiling() {
        let response = AssetStoreStatusResponse::available(AssetStats {
            object_count: u64::MAX,
            active_bytes: u64::MAX,
            reference_count: u64::MAX,
            missing_count: u64::MAX,
            quarantined_count: u64::MAX,
            staging_count: u64::MAX,
        });
        let bytes = serde_json::to_vec(&response).expect("maximum status bytes");

        assert!(bytes.len() <= MAX_ASSET_STATUS_RESPONSE_BYTES);
        assert_eq!(enforce_ipc_ceiling(response).error_code, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unavailable_path_returns_only_a_fixed_redacted_code() {
        let state = AssetStoreState::from_asset_root(None);
        assert!(state.backend.get().is_none());

        let response = state.status().await;
        let serialized = serde_json::to_string(&response).expect("unavailable status JSON");

        assert!(!response.available);
        assert_eq!(response.error_code, Some("ASSET_PATH_UNAVAILABLE"));
        assert!(response.stats.is_none());
        assert!(!serialized.contains("\"path\""));
        assert!(serialized.len() <= MAX_ASSET_STATUS_RESPONSE_BYTES);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn future_schema_is_closed_and_redacted() {
        let directory = tempdir().expect("temporary asset root");
        let asset_root = directory.path().join("private-future-assets");
        drop(
            AssetStore::open(&asset_root, product_asset_limits().expect("product limits"))
                .expect("initialize catalog"),
        );
        let connection = Connection::open(asset_root.join("assets.sqlite3")).expect("catalog");
        connection
            .pragma_update(None, "user_version", CURRENT_ASSET_SCHEMA_VERSION + 1)
            .expect("future schema");
        drop(connection);

        let response = AssetStoreState::from_asset_root(Some(asset_root.clone()))
            .status()
            .await;
        let serialized = serde_json::to_string(&response).expect("future status JSON");

        assert!(!response.available);
        assert_eq!(response.error_code, Some("ASSET_SCHEMA_INCOMPATIBLE"));
        assert!(response.stats.is_none());
        assert!(!serialized.contains("private-future-assets"));
        assert!(!serialized.contains("\"found\""));
        assert!(!serialized.contains("\"reason\""));
        assert!(serialized.len() <= MAX_ASSET_STATUS_RESPONSE_BYTES);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn corrupt_catalog_is_closed_and_redacted() {
        let directory = tempdir().expect("temporary asset root");
        let asset_root = directory.path().join("private-corrupt-assets");
        std::fs::create_dir_all(&asset_root).expect("asset root");
        std::fs::write(
            asset_root.join("assets.sqlite3"),
            b"not a sqlite catalog; secret fixture detail",
        )
        .expect("corrupt catalog");

        let response = AssetStoreState::from_asset_root(Some(asset_root))
            .status()
            .await;
        let serialized = serde_json::to_string(&response).expect("corrupt status JSON");

        assert!(!response.available);
        assert_eq!(response.error_code, Some("ASSET_STORE_UNAVAILABLE"));
        assert!(response.stats.is_none());
        assert!(!serialized.contains("private-corrupt-assets"));
        assert!(!serialized.contains("secret fixture detail"));
        assert!(serialized.len() <= MAX_ASSET_STATUS_RESPONSE_BYTES);
    }

    #[test]
    fn attacker_controlled_asset_errors_are_mapped_without_path_or_reason() {
        let error = AssetError::UnsafeFilesystem {
            path: "/private/secret/card.png".to_owned(),
            reason: "attacker supplied detail".to_owned(),
        };
        let response = AssetStoreStatusResponse::unavailable(map_asset_error(&error));
        let serialized = serde_json::to_string(&response).expect("redacted status JSON");

        assert_eq!(response.error_code, Some("ASSET_FILESYSTEM_UNSAFE"));
        assert!(!serialized.contains("/private/secret"));
        assert!(!serialized.contains("attacker supplied detail"));
        assert!(serialized.len() <= MAX_ASSET_STATUS_RESPONSE_BYTES);
    }
}
