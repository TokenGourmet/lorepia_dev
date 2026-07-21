use std::{
    collections::BTreeSet,
    io::{self, Read},
    path::{Path, PathBuf},
};

use lorepia_assets::{
    AssetLimits, AssetMime, AssetOwner, AssetStore, IngestRequest, MAX_PAGE_SIZE,
};
use serde::Serialize;

use crate::util::{MIB, Result, emit_receipt, ensure_free_space, invalid, prepare_new_directory};

const MIN_WAV_BYTES: u64 = 44;
const MAX_WAV_BYTES: u64 = u32::MAX as u64 + 8;

#[derive(Debug)]
pub struct AssetOptions {
    pub count: u64,
    pub target_active_bytes: u64,
    pub duplicate_rate: f64,
    pub seed: u64,
    pub output: PathBuf,
    pub receipt: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct AssetPlan {
    count: u64,
    unique_count: u64,
    duplicate_count: u64,
    unique_sizes: Vec<u64>,
    target_active_bytes: u64,
}

impl AssetPlan {
    fn new(count: u64, target_active_bytes: u64, duplicate_rate: f64) -> Result<Self> {
        if !duplicate_rate.is_finite() || !(0.0..=1.0).contains(&duplicate_rate) {
            return Err(invalid("duplicate rate must be finite and between 0 and 1"));
        }
        if count > u64::from(u32::MAX) {
            return Err(invalid(
                "--count exceeds the deterministic WAV fixture namespace",
            ));
        }
        if count == 0 {
            if target_active_bytes != 0 {
                return Err(invalid("--total must be zero when --count is zero"));
            }
            return Ok(Self {
                count,
                unique_count: 0,
                duplicate_count: 0,
                unique_sizes: Vec::new(),
                target_active_bytes,
            });
        }
        let requested_duplicates = ((count as f64) * duplicate_rate).round() as u64;
        let duplicate_count = requested_duplicates.min(count - 1);
        let unique_count = count - duplicate_count;
        let minimum = unique_count
            .checked_mul(MIN_WAV_BYTES)
            .ok_or_else(|| invalid("minimum asset size overflowed"))?;
        let maximum = unique_count
            .checked_mul(MAX_WAV_BYTES)
            .ok_or_else(|| invalid("maximum asset size overflowed"))?;
        if !(minimum..=maximum).contains(&target_active_bytes) {
            return Err(invalid(format!(
                "--total must be between {minimum} and {maximum} bytes for {unique_count} unique WAV fixtures"
            )));
        }
        let unique_len = usize::try_from(unique_count)
            .map_err(|_| invalid("unique asset count exceeds this platform's range"))?;
        let mut unique_sizes = vec![MIN_WAV_BYTES; unique_len];
        let remaining = target_active_bytes - minimum;
        let per_object = remaining / unique_count;
        let remainder = remaining % unique_count;
        for (index, size) in unique_sizes.iter_mut().enumerate() {
            *size = size
                .checked_add(per_object)
                .and_then(|value| value.checked_add(u64::from((index as u64) < remainder)))
                .ok_or_else(|| invalid("asset fixture size overflowed"))?;
            if *size > MAX_WAV_BYTES {
                return Err(invalid(
                    "asset fixture exceeds the RIFF/WAV 32-bit size limit",
                ));
            }
        }
        Ok(Self {
            count,
            unique_count,
            duplicate_count,
            unique_sizes,
            target_active_bytes,
        })
    }

    fn source_index(&self, attempt: u64, seed: u64) -> u64 {
        if attempt < self.unique_count {
            attempt
        } else {
            seed.wrapping_add(attempt - self.unique_count) % self.unique_count
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetReceipt {
    artifact_kind: &'static str,
    tool_version: &'static str,
    seed: u64,
    output: String,
    ingest_attempts: u64,
    unique_objects: u64,
    duplicate_attempts: u64,
    requested_duplicate_rate: f64,
    actual_duplicate_rate: f64,
    duplicate_rate_count_tolerance: u64,
    target_active_bytes: u64,
    actual_active_bytes: u64,
    active_byte_tolerance: u64,
    logical_bytes_read: u64,
    reference_count: u64,
    shard_prefix_count: usize,
    staging_count: u64,
    missing_count: u64,
    quarantined_count: u64,
    preflight_required_bytes: u64,
    preflight_available_bytes: u64,
    cas_api_used: bool,
    valid_fixture_mime: &'static str,
}

pub fn generate(options: AssetOptions) -> Result<()> {
    let plan = AssetPlan::new(
        options.count,
        options.target_active_bytes,
        options.duplicate_rate,
    )?;
    if plan.target_active_bytes > i64::MAX as u64 {
        return Err(invalid(
            "--total exceeds the asset catalog's signed 64-bit limit",
        ));
    }
    let preflight_required = plan
        .target_active_bytes
        .checked_add(plan.target_active_bytes / 5)
        .and_then(|bytes| bytes.checked_add(64 * MIB))
        .ok_or_else(|| invalid("asset free-space preflight overflowed"))?;
    let parent = options.output.parent().unwrap_or_else(|| Path::new("."));
    let preflight_available = ensure_free_space(parent, preflight_required)?;
    let output = prepare_new_directory(&options.output)?;

    let maximum_object = plan
        .unique_sizes
        .iter()
        .copied()
        .max()
        .unwrap_or(MIN_WAV_BYTES);
    let quota = plan.target_active_bytes.max(maximum_object);
    let limits = AssetLimits::new(maximum_object, quota.max(MIN_WAV_BYTES))?;
    let store = AssetStore::open(&output, limits)?;

    let mut observed_duplicates = 0_u64;
    let mut logical_bytes_read = 0_u64;
    for attempt in 0..plan.count {
        let source_index = plan.source_index(attempt, options.seed);
        let size = plan.unique_sizes[source_index as usize];
        let owner = AssetOwner::new("loadgen", format!("attempt-{attempt:010}"))?;
        let request = IngestRequest::new(AssetMime::Wav)
            .with_source_name(format!("synthetic-{source_index:010}.wav"))?
            .with_owner(owner);
        let mut fixture = DeterministicWav::new(size, options.seed, source_index)?;
        let outcome = store.ingest_uncancelled(&mut fixture, request)?;
        observed_duplicates += u64::from(outcome.deduplicated);
        logical_bytes_read = logical_bytes_read
            .checked_add(size)
            .ok_or_else(|| invalid("logical asset bytes overflowed"))?;
    }

    if observed_duplicates != plan.duplicate_count {
        return Err(invalid(format!(
            "CAS deduplication count mismatch: planned {}, observed {observed_duplicates}",
            plan.duplicate_count
        )));
    }
    let stats = store.verify_catalog_ledger()?;
    if stats.object_count != plan.unique_count
        || stats.active_bytes != plan.target_active_bytes
        || stats.reference_count != plan.count
    {
        return Err(invalid("asset catalog count/byte reconciliation failed"));
    }

    let mut cursor = None;
    let mut shards = BTreeSet::new();
    let mut listed = 0_u64;
    loop {
        let page = store.list_objects(cursor.as_ref(), MAX_PAGE_SIZE)?;
        for object in &page.objects {
            let hash = object.hash.as_str();
            let expected = format!("objects/{}/{}/{}", &hash[..2], &hash[2..4], hash);
            if object.relative_path != expected {
                return Err(invalid(
                    "asset object does not use the product CAS shard layout",
                ));
            }
            shards.insert(hash[..2].to_owned());
            listed += 1;
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    if listed != plan.unique_count {
        return Err(invalid(
            "keyset asset listing did not reconcile with the ledger",
        ));
    }

    let receipt = AssetReceipt {
        artifact_kind: "LOREPIA_PRODUCT_CAS_LOAD_FIXTURE",
        tool_version: env!("CARGO_PKG_VERSION"),
        seed: options.seed,
        output: output.display().to_string(),
        ingest_attempts: plan.count,
        unique_objects: plan.unique_count,
        duplicate_attempts: observed_duplicates,
        requested_duplicate_rate: options.duplicate_rate,
        actual_duplicate_rate: if plan.count == 0 {
            0.0
        } else {
            observed_duplicates as f64 / plan.count as f64
        },
        duplicate_rate_count_tolerance: 1,
        target_active_bytes: plan.target_active_bytes,
        actual_active_bytes: stats.active_bytes,
        active_byte_tolerance: 0,
        logical_bytes_read,
        reference_count: stats.reference_count,
        shard_prefix_count: shards.len(),
        staging_count: stats.staging_count,
        missing_count: stats.missing_count,
        quarantined_count: stats.quarantined_count,
        preflight_required_bytes: preflight_required,
        preflight_available_bytes: preflight_available,
        cas_api_used: true,
        valid_fixture_mime: "audio/wav",
    };
    emit_receipt(options.receipt.as_deref(), &receipt)
}

#[derive(Debug)]
struct DeterministicWav {
    size: u64,
    position: u64,
    seed: u64,
    index: u64,
    header: [u8; MIN_WAV_BYTES as usize],
}

impl DeterministicWav {
    fn new(size: u64, seed: u64, index: u64) -> Result<Self> {
        if !(MIN_WAV_BYTES..=MAX_WAV_BYTES).contains(&size) {
            return Err(invalid("WAV fixture size is outside its valid RIFF range"));
        }
        let mut header = [0_u8; MIN_WAV_BYTES as usize];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&u32::try_from(size - 8)?.to_le_bytes());
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16_u32.to_le_bytes());
        header[20..22].copy_from_slice(&1_u16.to_le_bytes());
        header[22..24].copy_from_slice(&1_u16.to_le_bytes());
        let unique_tag = (seed as u32) ^ u32::try_from(index)?;
        header[24..28].copy_from_slice(&unique_tag.to_le_bytes());
        header[28..32].copy_from_slice(&unique_tag.to_le_bytes());
        header[32..34].copy_from_slice(&1_u16.to_le_bytes());
        header[34..36].copy_from_slice(&8_u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&u32::try_from(size - MIN_WAV_BYTES)?.to_le_bytes());
        Ok(Self {
            size,
            position: 0,
            seed,
            index,
            header,
        })
    }
}

impl Read for DeterministicWav {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.size.saturating_sub(self.position);
        let count = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        if count == 0 {
            return Ok(0);
        }
        for (offset, byte) in buffer[..count].iter_mut().enumerate() {
            let absolute = self.position + offset as u64;
            if absolute < MIN_WAV_BYTES {
                *byte = self.header[absolute as usize];
            } else {
                *byte = fixture_byte(self.seed, self.index, absolute - MIN_WAV_BYTES);
            }
        }
        self.position += count as u64;
        Ok(count)
    }
}

fn fixture_byte(seed: u64, index: u64, position: u64) -> u8 {
    let mut value = seed
        ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ position.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (value ^ (value >> 31)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_plan_scales_to_one_hundred_thousand_without_materializing_assets() {
        let plan = AssetPlan::new(100_000, 10 * 1024 * 1024, 0.9).unwrap();
        assert_eq!(plan.count, 100_000);
        assert_eq!(plan.unique_count, 10_000);
        assert_eq!(plan.duplicate_count, 90_000);
        assert_eq!(plan.unique_sizes.iter().sum::<u64>(), 10 * 1024 * 1024);
    }

    #[test]
    fn metadata_plan_handles_zero_and_one_count_edges() {
        let empty = AssetPlan::new(0, 0, 0.0).unwrap();
        assert_eq!(empty.unique_count, 0);
        assert!(empty.unique_sizes.is_empty());

        let one = AssetPlan::new(1, MIN_WAV_BYTES, 1.0).unwrap();
        assert_eq!(one.unique_count, 1);
        assert_eq!(one.duplicate_count, 0);
        assert_eq!(one.unique_sizes, vec![MIN_WAV_BYTES]);
    }

    #[test]
    fn wav_reader_is_valid_sized_and_chunk_independent() {
        let mut once = DeterministicWav::new(1_001, 42, 7).unwrap();
        let mut all = Vec::new();
        once.read_to_end(&mut all).unwrap();
        let mut chunked = DeterministicWav::new(1_001, 42, 7).unwrap();
        let mut chunks = Vec::new();
        let mut buffer = [0_u8; 13];
        loop {
            let read = chunked.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            chunks.extend_from_slice(&buffer[..read]);
        }
        assert_eq!(all, chunks);
        assert_eq!(all.len(), 1_001);
        assert_eq!(&all[..4], b"RIFF");
        assert_eq!(&all[8..12], b"WAVE");
    }

    #[test]
    fn uses_real_cas_and_deduplicates_valid_fixtures() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("assets");
        let receipt = directory.path().join("assets-receipt.json");
        generate(AssetOptions {
            count: 10,
            target_active_bytes: 4_096,
            duplicate_rate: 0.6,
            seed: 42,
            output: output.clone(),
            receipt: Some(receipt),
        })
        .unwrap();
        assert!(output.join("assets.sqlite3").is_file());
        assert!(output.join("objects").is_dir());
    }
}
