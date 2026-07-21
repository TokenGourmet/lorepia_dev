use std::{
    io::{Cursor, Read, Write},
    sync::{Arc, mpsc},
    thread,
};

use crc32fast::Hasher as Crc32;
use lorepia_assets::{AssetLimits, AssetMime, AssetOwner, AssetStore, IngestRequest};
use lorepia_import::{ImportErrorCode, ImportLimits, ImportService};
use tempfile::TempDir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[derive(Clone)]
struct FixtureEntry<'a> {
    name: &'a str,
    bytes: &'a [u8],
    compression: CompressionMethod,
}

fn fixture() -> (TempDir, ImportService, AssetOwner) {
    fixture_with_limits(ImportLimits::default())
}

fn fixture_with_limits(limits: ImportLimits) -> (TempDir, ImportService, AssetOwner) {
    let (temp, service, owner, _) = fixture_with_store(limits);
    (temp, service, owner)
}

fn fixture_with_store(limits: ImportLimits) -> (TempDir, ImportService, AssetOwner, AssetStore) {
    let temp = tempfile::tempdir().expect("temporary root");
    let asset_limits = AssetLimits::new(128 * 1024 * 1024, 512 * 1024 * 1024)
        .expect("asset limits")
        .with_image_limits(16_384, 16_384, 67_108_864)
        .expect("image limits");
    let store = AssetStore::open(temp.path().join("assets"), asset_limits).expect("asset store");
    let service = ImportService::open(temp.path().join("imports"), store.clone(), limits)
        .expect("import service");
    let owner = AssetOwner::new("character", "fixture-card").expect("owner");
    (temp, service, owner, store)
}

fn import_bytes(
    service: &ImportService,
    owner: &AssetOwner,
    bytes: &[u8],
) -> lorepia_import::Result<lorepia_import::ImportReceipt> {
    service.import_reader(&mut Cursor::new(bytes), owner.clone(), || false)
}

fn valid_png() -> Vec<u8> {
    valid_png_with_color([0x22, 0x44, 0x66, 0xff])
}

fn valid_png_with_color(color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(&color).expect("PNG data");
    }
    bytes
}

fn zip_bytes(entries: &[FixtureEntry<'_>]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    for entry in entries {
        let options = SimpleFileOptions::default().compression_method(entry.compression);
        writer.start_file(entry.name, options).expect("ZIP entry");
        writer.write_all(entry.bytes).expect("ZIP payload");
    }
    writer.finish().expect("finish ZIP").into_inner()
}

fn insert_png_chunk(bytes: &mut Vec<u8>, offset: usize, kind: &[u8; 4], payload: &[u8]) {
    let mut chunk = Vec::with_capacity(payload.len() + 12);
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(payload);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(payload);
    chunk.extend_from_slice(&crc.finalize().to_be_bytes());
    bytes.splice(offset..offset, chunk);
}

fn staging_is_empty(temp: &TempDir) -> bool {
    std::fs::read_dir(temp.path().join("imports"))
        .expect("staging root")
        .next()
        .is_none()
}

#[test]
fn admits_png_and_archive_assets_but_only_records_executable_metadata() {
    let (temp, service, owner) = fixture();
    let png = valid_png();
    let direct = import_bytes(&service, &owner, &png).expect("direct PNG");
    assert_eq!(direct.counts.accepted, 1);
    assert_eq!(direct.assets.len(), 1);
    assert_eq!(direct.executable_entries_executed, 0);

    let archive = zip_bytes(&[
        FixtureEntry {
            name: "manifest.json",
            bytes: br#"{"name":"Self-authored fixture"}"#,
            compression: CompressionMethod::Deflated,
        },
        FixtureEntry {
            name: "assets/card.png",
            bytes: &png,
            compression: CompressionMethod::Deflated,
        },
        FixtureEntry {
            name: "scripts/behavior.js",
            bytes: b"throw new Error('must never execute')",
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "scripts/behavior.lua",
            bytes: b"while true do end",
            compression: CompressionMethod::Stored,
        },
    ]);
    let receipt = import_bytes(&service, &owner, &archive).expect("safe archive");
    assert_eq!(receipt.counts.accepted, 2);
    assert_eq!(receipt.counts.quarantined, 2);
    assert_eq!(receipt.counts.rejected, 0);
    assert_eq!(receipt.assets.len(), 1);
    assert_eq!(receipt.metadata.len(), 1);
    assert_eq!(receipt.quarantined.len(), 2);
    assert!(receipt.quarantined.iter().all(|entry| {
        !entry.executable
            && entry.disposition == "INERT_QUARANTINED"
            && entry.policy == "DISABLED_BY_SECURITY_POLICY"
    }));
    assert_eq!(receipt.executable_entries_executed, 0);
    assert!(staging_is_empty(&temp));
}

#[test]
fn metadata_only_and_duplicate_content_batches_close_the_exact_session_contract() {
    let (temp, service, owner, store) = fixture_with_store(ImportLimits::default());
    let metadata_only = zip_bytes(&[FixtureEntry {
        name: "manifest.json",
        bytes: br#"{"name":"Metadata only"}"#,
        compression: CompressionMethod::Stored,
    }]);
    let metadata_receipt = import_bytes(&service, &owner, &metadata_only)
        .expect("metadata-only archive remains valid");
    assert!(metadata_receipt.assets.is_empty());
    assert_eq!(metadata_receipt.metadata.len(), 1);
    assert_eq!(store.stats().expect("metadata stats").reference_count, 0);

    let png = valid_png();
    let duplicate_content = zip_bytes(&[
        FixtureEntry {
            name: "assets/first.png",
            bytes: &png,
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "assets/second.png",
            bytes: &png,
            compression: CompressionMethod::Stored,
        },
    ]);
    let duplicate_receipt = import_bytes(&service, &owner, &duplicate_content)
        .expect("two logical paths may share one content hash");
    assert_eq!(duplicate_receipt.assets.len(), 2);
    assert_eq!(
        duplicate_receipt.assets[0].hash,
        duplicate_receipt.assets[1].hash
    );
    assert_eq!(store.stats().expect("dedupe stats").reference_count, 1);
    assert!(staging_is_empty(&temp));
}

#[test]
fn reserved_or_invalid_final_owners_are_rejected_before_source_io() {
    let (temp, service, _, store) = fixture_with_store(ImportLimits::default());
    let reserved = AssetOwner::new("lorepia-import-session", "attacker-final")
        .expect("reserved owner is syntactically valid");
    assert_eq!(
        service
            .import_reader(&mut Cursor::new(valid_png()), reserved.clone(), || false)
            .expect_err("reserved namespace")
            .code,
        ImportErrorCode::AssetRejected
    );
    assert_eq!(
        service
            .import_path(
                temp.path().join("source-must-not-be-opened.png"),
                reserved,
                || false,
            )
            .expect_err("reserved namespace precedes source open")
            .code,
        ImportErrorCode::AssetRejected
    );
    let forged = AssetOwner {
        owner_type: "UPPERCASE-IS-INVALID".to_owned(),
        owner_id: "forged".to_owned(),
    };
    assert_eq!(
        service
            .import_reader(&mut Cursor::new(valid_png()), forged, || false)
            .expect_err("forged invalid owner")
            .code,
        ImportErrorCode::AssetRejected
    );
    let stats = store.stats().expect("asset stats");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.reference_count, 0);
    assert!(staging_is_empty(&temp));
}

#[test]
fn rejects_traversal_portable_aliases_and_path_collisions() {
    let (temp, service, owner) = fixture();
    let png = valid_png();
    for name in [
        "../escape.png",
        "nested/../../escape.png",
        "/absolute.png",
        "C:/drive.png",
        "folder\\escape.png",
        "CON.png",
        "cafe\u{301}.png",
    ] {
        let archive = zip_bytes(&[FixtureEntry {
            name,
            bytes: &png,
            compression: CompressionMethod::Stored,
        }]);
        let error = import_bytes(&service, &owner, &archive).expect_err("unsafe path");
        assert_eq!(error.code, ImportErrorCode::UnsafePath, "{name}");
    }

    let duplicate = zip_bytes(&[
        FixtureEntry {
            name: "Assets/Card.png",
            bytes: &png,
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "assets/card.PNG",
            bytes: &png,
            compression: CompressionMethod::Stored,
        },
    ]);
    assert_eq!(
        import_bytes(&service, &owner, &duplicate)
            .expect_err("case collision")
            .code,
        ImportErrorCode::DuplicatePath
    );
    assert!(staging_is_empty(&temp));
}

#[test]
fn rejects_symlink_nested_archive_encryption_and_header_divergence() {
    let (temp, service, owner) = fixture();

    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    writer
        .add_symlink(
            "assets/link.png",
            "../../escape",
            SimpleFileOptions::default(),
        )
        .expect("symlink fixture");
    let symlink = writer.finish().expect("symlink ZIP").into_inner();
    assert_eq!(
        import_bytes(&service, &owner, &symlink)
            .expect_err("symlink")
            .code,
        ImportErrorCode::UnsafeEntryType
    );

    let inner = zip_bytes(&[]);
    let nested = zip_bytes(&[FixtureEntry {
        name: "nested.zip",
        bytes: &inner,
        compression: CompressionMethod::Stored,
    }]);
    assert_eq!(
        import_bytes(&service, &owner, &nested)
            .expect_err("nested archive")
            .code,
        ImportErrorCode::UnsupportedFileType
    );

    let active_content = zip_bytes(&[FixtureEntry {
        name: "assets/active.svg",
        bytes: br#"<svg onload="fetch('https://invalid')"/>"#,
        compression: CompressionMethod::Stored,
    }]);
    assert_eq!(
        import_bytes(&service, &owner, &active_content)
            .expect_err("SVG active content")
            .code,
        ImportErrorCode::UnsupportedFileType
    );

    let mut divergent = zip_bytes(&[FixtureEntry {
        name: "manifest.json",
        bytes: b"{}",
        compression: CompressionMethod::Stored,
    }]);
    divergent[14] ^= 0x01;
    assert_eq!(
        import_bytes(&service, &owner, &divergent)
            .expect_err("local/central CRC divergence")
            .code,
        ImportErrorCode::ArchiveMalformed
    );

    let mut encrypted = zip_bytes(&[FixtureEntry {
        name: "manifest.json",
        bytes: b"{}",
        compression: CompressionMethod::Stored,
    }]);
    encrypted[6] |= 0x01;
    let central = find_signature(&encrypted, b"PK\x01\x02");
    encrypted[central + 8] |= 0x01;
    assert_eq!(
        import_bytes(&service, &owner, &encrypted)
            .expect_err("encrypted entry")
            .code,
        ImportErrorCode::UnsafeEntryType
    );
    assert!(staging_is_empty(&temp));
}

#[test]
fn rejects_zip64_data_descriptors_overlaps_and_unsupported_methods() {
    let (temp, service, owner) = fixture();
    let base = zip_bytes(&[FixtureEntry {
        name: "manifest.json",
        bytes: b"{}",
        compression: CompressionMethod::Stored,
    }]);
    let central = find_signature(&base, b"PK\x01\x02");

    let mut zip64 = base.clone();
    zip64[central + 20..central + 28].fill(0xff);
    assert_eq!(
        import_bytes(&service, &owner, &zip64)
            .expect_err("ZIP64 without bounded product support")
            .code,
        ImportErrorCode::ArchiveMalformed
    );

    let mut descriptor = base.clone();
    descriptor[6] |= 1 << 3;
    descriptor[central + 8] |= 1 << 3;
    assert_eq!(
        import_bytes(&service, &owner, &descriptor)
            .expect_err("data descriptor")
            .code,
        ImportErrorCode::ArchiveMalformed
    );

    let mut unsupported = base.clone();
    unsupported[8..10].copy_from_slice(&99u16.to_le_bytes());
    unsupported[central + 10..central + 12].copy_from_slice(&99u16.to_le_bytes());
    assert_eq!(
        import_bytes(&service, &owner, &unsupported)
            .expect_err("unsupported compression")
            .code,
        ImportErrorCode::UnsupportedCompression
    );

    let mut overlap = zip_bytes(&[
        FixtureEntry {
            name: "first.json",
            bytes: b"{}",
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "other.json",
            bytes: b"[]",
            compression: CompressionMethod::Stored,
        },
    ]);
    let central_headers = find_signatures(&overlap, b"PK\x01\x02");
    assert_eq!(central_headers.len(), 2);
    overlap[central_headers[1] + 42..central_headers[1] + 46].fill(0);
    assert_eq!(
        import_bytes(&service, &owner, &overlap)
            .expect_err("overlapping/reused local entry")
            .code,
        ImportErrorCode::ArchiveMalformed
    );
    assert!(staging_is_empty(&temp));
}

#[test]
fn rejects_central_and_local_extra_fields_above_the_configured_allocation_ceiling() {
    let limits = ImportLimits {
        max_zip_extra_bytes: 8,
        ..ImportLimits::default()
    };
    let (temp, service, owner) = fixture_with_limits(limits);
    let base = zip_bytes(&[FixtureEntry {
        name: "manifest.json",
        bytes: b"{}",
        compression: CompressionMethod::Stored,
    }]);
    let central = find_signature(&base, b"PK\x01\x02");

    let mut central_extra = base.clone();
    central_extra[central + 30..central + 32].copy_from_slice(&9u16.to_le_bytes());
    assert_eq!(
        import_bytes(&service, &owner, &central_extra)
            .expect_err("central extra allocation ceiling")
            .code,
        ImportErrorCode::ArchiveMalformed
    );

    let mut local_extra = base;
    local_extra[28..30].copy_from_slice(&9u16.to_le_bytes());
    assert_eq!(
        import_bytes(&service, &owner, &local_extra)
            .expect_err("local extra allocation ceiling")
            .code,
        ImportErrorCode::ArchiveMalformed
    );
    assert!(staging_is_empty(&temp));
}

#[cfg(unix)]
#[test]
fn import_path_rejects_a_final_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let (temp, service, owner) = fixture();
    let target = temp.path().join("real-card.png");
    let alias = temp.path().join("alias-card.png");
    std::fs::write(&target, valid_png()).expect("source fixture");
    symlink(&target, &alias).expect("source symlink");
    let error = service
        .import_path(&alias, owner, || false)
        .expect_err("symlink source");
    assert_eq!(error.code, ImportErrorCode::UnsafeEntryType);
    assert!(staging_is_empty(&temp));
}

#[test]
fn rejects_entry_count_size_and_compression_bombs_before_publish() {
    let limits = ImportLimits {
        max_archive_entries: 1,
        max_compression_ratio: 10,
        ..ImportLimits::default()
    };
    let (temp, service, owner) = fixture_with_limits(limits);
    let two = zip_bytes(&[
        FixtureEntry {
            name: "a.json",
            bytes: b"{}",
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "b.json",
            bytes: b"{}",
            compression: CompressionMethod::Stored,
        },
    ]);
    assert_eq!(
        import_bytes(&service, &owner, &two)
            .expect_err("entry count")
            .code,
        ImportErrorCode::EntryCountLimit
    );

    let zeros = vec![0u8; 1024 * 1024];
    let bomb = zip_bytes(&[FixtureEntry {
        name: "manifest.json",
        bytes: &zeros,
        compression: CompressionMethod::Deflated,
    }]);
    let (uncompressed, compressed) = zip_sizes(&bomb, 0);
    assert!(uncompressed > compressed.saturating_mul(1_000));
    assert_eq!(
        import_bytes(&service, &owner, &bomb)
            .expect_err("compression ratio")
            .code,
        ImportErrorCode::CompressionRatioLimit
    );
    assert!(staging_is_empty(&temp));

    let size_limits = ImportLimits {
        max_entry_bytes: 1_024,
        max_total_uncompressed_bytes: 1_024,
        ..ImportLimits::default()
    };
    let (_size_temp, size_service, size_owner) = fixture_with_limits(size_limits);
    let oversized = vec![b'x'; 1_025];
    let oversized_archive = zip_bytes(&[FixtureEntry {
        name: "large.json",
        bytes: &oversized,
        compression: CompressionMethod::Stored,
    }]);
    assert_eq!(
        import_bytes(&size_service, &size_owner, &oversized_archive)
            .expect_err("entry byte limit")
            .code,
        ImportErrorCode::EntrySizeLimit
    );
    let aggregate = vec![b'x'; 600];
    let aggregate_archive = zip_bytes(&[
        FixtureEntry {
            name: "a.json",
            bytes: &aggregate,
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "b.json",
            bytes: &aggregate,
            compression: CompressionMethod::Stored,
        },
    ]);
    assert_eq!(
        import_bytes(&size_service, &size_owner, &aggregate_archive)
            .expect_err("aggregate byte limit")
            .code,
        ImportErrorCode::TotalSizeLimit
    );
}

#[test]
fn rejects_png_crc_truncation_order_dimensions_animation_and_metadata_bombs() {
    let (temp, service, owner) = fixture();

    let mut bad_crc = valid_png();
    bad_crc[29] ^= 0x01;
    assert_eq!(
        import_bytes(&service, &owner, &bad_crc)
            .expect_err("bad CRC")
            .code,
        ImportErrorCode::PngMalformed
    );

    let mut truncated = valid_png();
    truncated.truncate(truncated.len() - 2);
    assert_eq!(
        import_bytes(&service, &owner, &truncated)
            .expect_err("truncated chunk")
            .code,
        ImportErrorCode::PngMalformed
    );

    let mut wrong_order = valid_png();
    insert_png_chunk(&mut wrong_order, 8, b"IDAT", b"x");
    assert_eq!(
        import_bytes(&service, &owner, &wrong_order)
            .expect_err("wrong chunk order")
            .code,
        ImportErrorCode::PngMalformed
    );

    let mut dimensions = valid_png();
    dimensions[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    let mut crc = Crc32::new();
    crc.update(b"IHDR");
    crc.update(&dimensions[16..29]);
    dimensions[29..33].copy_from_slice(&crc.finalize().to_be_bytes());
    assert_eq!(
        import_bytes(&service, &owner, &dimensions)
            .expect_err("dimension bomb")
            .code,
        ImportErrorCode::PngDimensionLimit
    );

    let mut animation = valid_png();
    insert_png_chunk(&mut animation, 33, b"acTL", &[0; 8]);
    assert_eq!(
        import_bytes(&service, &owner, &animation)
            .expect_err("animation")
            .code,
        ImportErrorCode::PngAnimationUnsupported
    );

    let limits = ImportLimits {
        max_png_metadata_bytes: 8,
        ..ImportLimits::default()
    };
    let (_metadata_temp, metadata_service, metadata_owner) = fixture_with_limits(limits);
    let mut metadata_bomb = valid_png();
    insert_png_chunk(&mut metadata_bomb, 33, b"eXIf", &[0; 9]);
    assert_eq!(
        import_bytes(&metadata_service, &metadata_owner, &metadata_bomb)
            .expect_err("metadata bomb")
            .code,
        ImportErrorCode::PngMetadataLimit
    );

    let mut icc_bomb = valid_png();
    insert_png_chunk(&mut icc_bomb, 33, b"iCCP", b"profile\0\0compressed");
    assert_eq!(
        import_bytes(&service, &owner, &icc_bomb)
            .expect_err("compressed ICC metadata")
            .code,
        ImportErrorCode::PngMetadataLimit
    );

    let mut trailing = valid_png();
    trailing.extend_from_slice(b"attacker-trailer");
    assert_eq!(
        import_bytes(&service, &owner, &trailing)
            .expect_err("trailing PNG bytes")
            .code,
        ImportErrorCode::PngMalformed
    );
    assert!(staging_is_empty(&temp));
}

#[test]
fn cancellation_revoked_reader_and_malformed_crc_leave_zero_staging() {
    let (temp, service, owner) = fixture();
    let png = valid_png();
    let mut checks = 0u32;
    let error = service
        .import_reader(&mut Cursor::new(&png), owner.clone(), || {
            checks += 1;
            checks > 1
        })
        .expect_err("cancelled copy");
    assert_eq!(error.code, ImportErrorCode::Cancelled);
    assert!(!error.cleanup_pending);
    assert!(staging_is_empty(&temp));

    let revoked = service
        .import_reader(&mut RevokedReader { first: true }, owner.clone(), || false)
        .expect_err("revoked source");
    assert_eq!(revoked.code, ImportErrorCode::StagingFailure);
    assert!(staging_is_empty(&temp));

    let mut corrupt_zip = zip_bytes(&[FixtureEntry {
        name: "manifest.json",
        bytes: br#"{"ok":true}"#,
        compression: CompressionMethod::Stored,
    }]);
    let data_start = zip_data_start(&corrupt_zip, 0);
    corrupt_zip[data_start] ^= 0xff;
    assert_eq!(
        import_bytes(&service, &owner, &corrupt_zip)
            .expect_err("ZIP payload CRC")
            .code,
        ImportErrorCode::ArchiveMalformed
    );
    assert!(staging_is_empty(&temp));
}

#[test]
fn later_mime_failure_rolls_back_every_new_active_object() {
    let (temp, service, owner, store) = fixture_with_store(ImportLimits::default());
    let first = valid_png_with_color([0x10, 0x20, 0x30, 0xff]);
    let mismatched = valid_png_with_color([0x40, 0x50, 0x60, 0xff]);
    let archive = zip_bytes(&[
        FixtureEntry {
            name: "assets/first.png",
            bytes: &first,
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "assets/not-really-jpeg.jpg",
            bytes: &mismatched,
            compression: CompressionMethod::Stored,
        },
    ]);

    assert_eq!(
        import_bytes(&service, &owner, &archive)
            .expect_err("second asset MIME mismatch")
            .code,
        ImportErrorCode::AssetRejected
    );
    let stats = store.stats().expect("asset stats");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.active_bytes, 0);
    assert_eq!(stats.reference_count, 0);
    assert!(staging_is_empty(&temp));
}

#[test]
fn cancellation_after_one_asset_ingest_rolls_back_the_session() {
    let (temp, service, owner, store) = fixture_with_store(ImportLimits::default());
    let first = valid_png_with_color([0x11, 0x22, 0x33, 0xff]);
    let second = valid_png_with_color([0x44, 0x55, 0x66, 0xff]);
    let archive = zip_bytes(&[
        FixtureEntry {
            name: "assets/first.png",
            bytes: &first,
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "assets/second.png",
            bytes: &second,
            compression: CompressionMethod::Stored,
        },
    ]);

    let observed_store = store.clone();
    let error = service
        .import_reader(&mut Cursor::new(archive), owner, move || {
            observed_store
                .stats()
                .is_ok_and(|stats| stats.reference_count == 1)
        })
        .expect_err("cancel after first admitted object");
    assert_eq!(error.code, ImportErrorCode::Cancelled);
    let stats = store.stats().expect("asset stats");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.active_bytes, 0);
    assert_eq!(stats.reference_count, 0);
    assert!(staging_is_empty(&temp));
}

#[test]
fn later_staged_file_io_failure_rolls_back_the_session() {
    let (temp, service, owner, store) = fixture_with_store(ImportLimits::default());
    let first = valid_png_with_color([0x12, 0x23, 0x34, 0xff]);
    let second = valid_png_with_color([0x45, 0x56, 0x67, 0xff]);
    let archive = zip_bytes(&[
        FixtureEntry {
            name: "assets/first.png",
            bytes: &first,
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "assets/second.png",
            bytes: &second,
            compression: CompressionMethod::Stored,
        },
    ]);

    let observed_store = store.clone();
    let import_root = temp.path().join("imports");
    let mut revoked = false;
    let error = service
        .import_reader(&mut Cursor::new(archive), owner, || {
            if !revoked
                && observed_store
                    .stats()
                    .is_ok_and(|stats| stats.reference_count == 1)
            {
                let session = std::fs::read_dir(&import_root)
                    .expect("import root")
                    .next()
                    .expect("active session")
                    .expect("session entry")
                    .path();
                std::fs::remove_file(session.join("entry-00000001.bin"))
                    .expect("revoke second staged file");
                revoked = true;
            }
            false
        })
        .expect_err("second staged file was revoked");
    assert_eq!(error.code, ImportErrorCode::StagingFailure);
    let stats = store.stats().expect("asset stats");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.active_bytes, 0);
    assert_eq!(stats.reference_count, 0);
    assert!(staging_is_empty(&temp));
}

#[test]
fn repeated_failures_preserve_a_preexisting_referenced_deduplicated_object() {
    let (temp, service, owner, store) = fixture_with_store(ImportLimits::default());
    let existing = valid_png_with_color([0x77, 0x66, 0x55, 0xff]);
    let baseline = import_bytes(&service, &owner, &existing).expect("baseline import");
    let baseline_hash = baseline.assets[0].hash.clone();
    let mismatched = valid_png_with_color([0x88, 0x99, 0xaa, 0xff]);
    let archive = zip_bytes(&[
        FixtureEntry {
            name: "assets/existing.png",
            bytes: &existing,
            compression: CompressionMethod::Stored,
        },
        FixtureEntry {
            name: "assets/not-a-jpeg.jpg",
            bytes: &mismatched,
            compression: CompressionMethod::Stored,
        },
    ]);

    for attempt in 0..16 {
        assert_eq!(
            import_bytes(&service, &owner, &archive)
                .expect_err("repeated later failure")
                .code,
            ImportErrorCode::AssetRejected,
            "attempt {attempt}"
        );
        let stats = store.stats().expect("stable stats");
        assert_eq!(stats.object_count, 1, "attempt {attempt}");
        assert_eq!(
            stats.active_bytes,
            existing.len() as u64,
            "attempt {attempt}"
        );
        assert_eq!(stats.reference_count, 1, "attempt {attempt}");
        assert!(
            store
                .get_object(&baseline_hash)
                .expect("baseline lookup")
                .is_some(),
            "attempt {attempt}"
        );
        assert!(staging_is_empty(&temp));
    }
}

#[test]
fn open_recovers_a_crashed_import_session_from_durable_owner_refs() {
    let temp = tempfile::tempdir().expect("temporary root");
    let asset_limits =
        AssetLimits::new(128 * 1024 * 1024, 512 * 1024 * 1024).expect("asset limits");
    let store = AssetStore::open(temp.path().join("assets"), asset_limits).expect("asset store");
    let session_id = "0123456789abcdef0123456789abcdef";
    let temporary_owner =
        AssetOwner::new("lorepia-import-session", session_id).expect("temporary owner");
    let mut source = Cursor::new(valid_png());
    store
        .ingest_uncancelled(
            &mut source,
            IngestRequest::new(AssetMime::Png).with_owner(temporary_owner),
        )
        .expect("interrupted ingest");
    let import_root = temp.path().join("imports");
    std::fs::create_dir_all(import_root.join(format!("{session_id}.partial")))
        .expect("stale staging session");
    assert!(store.stats().expect("interrupted stats").active_bytes > 0);

    ImportService::open(import_root.clone(), store.clone(), ImportLimits::default())
        .expect("open performs recovery");
    let stats = store.stats().expect("recovered stats");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.active_bytes, 0);
    assert_eq!(stats.reference_count, 0);
    assert!(
        std::fs::read_dir(import_root)
            .expect("import root")
            .next()
            .is_none()
    );
}

#[test]
fn second_service_open_preserves_a_live_blocked_import_session() {
    let temp = tempfile::tempdir().expect("temporary root");
    let asset_limits =
        AssetLimits::new(128 * 1024 * 1024, 512 * 1024 * 1024).expect("asset limits");
    let store = AssetStore::open(temp.path().join("assets"), asset_limits).expect("asset store");
    let import_root = temp.path().join("imports");
    let first_service =
        ImportService::open(import_root.clone(), store.clone(), ImportLimits::default())
            .expect("first service");
    let owner = AssetOwner::new("character", "concurrent-open").expect("owner");
    let (temporary_ref_tx, temporary_ref_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let observed_store = store.clone();
    let first = thread::spawn(move || {
        let mut blocked_after_ingest = false;
        first_service.import_reader(&mut Cursor::new(valid_png()), owner, move || {
            if !blocked_after_ingest
                && observed_store
                    .stats()
                    .is_ok_and(|stats| stats.reference_count == 1)
            {
                blocked_after_ingest = true;
                temporary_ref_tx
                    .send(())
                    .expect("announce temporary reference");
                release_rx.recv().expect("release promotion barrier");
            }
            false
        })
    });
    temporary_ref_rx
        .recv()
        .expect("first import is blocked after asset ingest and before promotion");

    let _second_service =
        ImportService::open(import_root.clone(), store.clone(), ImportLimits::default())
            .expect("second open preserves live session");
    assert_eq!(
        std::fs::read_dir(&import_root)
            .expect("import root")
            .count(),
        1,
        "the live staging directory must not be reclaimed"
    );

    release_tx.send(()).expect("release first import");
    let receipt = first
        .join()
        .expect("first thread")
        .expect("first import remains valid");
    assert_eq!(receipt.assets.len(), 1);
    assert!(
        store
            .get_object(&receipt.assets[0].hash)
            .expect("object lookup")
            .is_some(),
        "a success receipt must still have its published object"
    );
    assert_eq!(store.stats().expect("stats").reference_count, 1);
    assert!(
        std::fs::read_dir(import_root)
            .expect("import root")
            .next()
            .is_none()
    );
}

#[test]
fn expired_durable_import_lease_is_reclaimed_with_its_staging_directory() {
    let temp = tempfile::tempdir().expect("temporary root");
    let asset_limits =
        AssetLimits::new(128 * 1024 * 1024, 512 * 1024 * 1024).expect("asset limits");
    let store = AssetStore::open(temp.path().join("assets"), asset_limits).expect("asset store");
    let session_id = "fedcba9876543210fedcba9876543210";
    let temporary_owner =
        AssetOwner::new("lorepia-import-session", session_id).expect("temporary owner");
    store
        .begin_temporary_owner_session(&temporary_owner)
        .expect("durable lease");
    let mut source = Cursor::new(valid_png());
    store
        .ingest_uncancelled(
            &mut source,
            IngestRequest::new(AssetMime::Png).with_owner(temporary_owner),
        )
        .expect("interrupted ingest");
    let import_root = temp.path().join("imports");
    std::fs::create_dir_all(import_root.join(format!("{session_id}.partial")))
        .expect("stale staging session");

    assert_eq!(
        store
            .recover_temporary_owner_type_before("lorepia-import-session", i64::MAX)
            .expect("claim expired lease"),
        1
    );
    ImportService::open(import_root.clone(), store.clone(), ImportLimits::default())
        .expect("staging cleanup after lease recovery");
    let stats = store.stats().expect("recovered stats");
    assert_eq!(stats.object_count, 0);
    assert_eq!(stats.reference_count, 0);
    assert!(
        std::fs::read_dir(import_root)
            .expect("import root")
            .next()
            .is_none()
    );
}

#[test]
fn one_of_one_hundred_concurrent_requests_is_admitted_and_the_rest_are_busy() {
    let (temp, service, owner) = fixture();
    let service = Arc::new(service);
    let png = valid_png();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first_service = Arc::clone(&service);
    let first_owner = owner.clone();
    let first_png = png.clone();
    let first = thread::spawn(move || {
        first_service.import_reader(
            &mut BlockingReader {
                inner: Cursor::new(first_png),
                started: Some(started_tx),
                release: release_rx,
            },
            first_owner,
            || false,
        )
    });
    started_rx.recv().expect("first request holds admission");

    let mut contenders = Vec::new();
    for _ in 0..99 {
        let contender_service = Arc::clone(&service);
        let contender_owner = owner.clone();
        let contender_png = png.clone();
        contenders.push(thread::spawn(move || {
            contender_service.import_reader(
                &mut Cursor::new(contender_png),
                contender_owner,
                || false,
            )
        }));
    }
    for contender in contenders {
        let error = contender
            .join()
            .expect("contender thread")
            .expect_err("bounded admission");
        assert_eq!(error.code, ImportErrorCode::Busy);
    }
    release_tx.send(()).expect("release admitted import");
    first
        .join()
        .expect("admitted thread")
        .expect("admitted import succeeds");
    assert!(staging_is_empty(&temp));
}

struct RevokedReader {
    first: bool,
}

impl Read for RevokedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.first {
            self.first = false;
            buffer[..4].copy_from_slice(b"PK\x03\x04");
            Ok(4)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture permission revoked",
            ))
        }
    }
}

struct BlockingReader {
    inner: Cursor<Vec<u8>>,
    started: Option<mpsc::Sender<()>>,
    release: mpsc::Receiver<()>,
}

impl Read for BlockingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(started) = self.started.take() {
            started.send(()).expect("signal admission");
            self.release.recv().expect("release admission");
        }
        self.inner.read(buffer)
    }
}

fn find_signature(bytes: &[u8], signature: &[u8]) -> usize {
    bytes
        .windows(signature.len())
        .position(|window| window == signature)
        .expect("signature")
}

fn find_signatures(bytes: &[u8], signature: &[u8]) -> Vec<usize> {
    bytes
        .windows(signature.len())
        .enumerate()
        .filter_map(|(index, window)| (window == signature).then_some(index))
        .collect()
}

fn zip_sizes(bytes: &[u8], index: usize) -> (u64, u64) {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("ZIP parser");
    let entry = archive.by_index_raw(index).expect("ZIP entry");
    (entry.size(), entry.compressed_size())
}

fn zip_data_start(bytes: &[u8], index: usize) -> usize {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("ZIP parser");
    archive
        .by_index(index)
        .expect("ZIP entry")
        .data_start()
        .expect("data offset") as usize
}
