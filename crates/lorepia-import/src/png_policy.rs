use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use crc32fast::Hasher as Crc32;
use sha2::{Digest, Sha256};

use crate::{AcceptedMetadata, ImportError, ImportErrorCode, ImportLimits, Result};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PngInfo {
    pub width: u32,
    pub height: u32,
}

pub(crate) fn validate_png<C>(
    path: &Path,
    logical_path: &str,
    limits: &ImportLimits,
    is_cancelled: &mut C,
) -> Result<(PngInfo, Vec<AcceptedMetadata>)>
where
    C: FnMut() -> bool,
{
    let metadata =
        std::fs::metadata(path).map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
    if !metadata.is_file() || metadata.len() > limits.max_png_bytes {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    }
    let file = File::open(path).map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
    let mut reader = BufReader::with_capacity(limits.copy_buffer_bytes, file);
    let mut signature = [0u8; 8];
    read_exact(&mut reader, &mut signature)?;
    if &signature != PNG_SIGNATURE {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    }

    let mut chunk_count = 0usize;
    let mut metadata_bytes = 0u64;
    let mut ihdr = None;
    let mut plte_seen = false;
    let mut idat_seen = false;
    let mut idat_ended = false;
    let mut iend_seen = false;
    let mut card_metadata = Vec::new();
    let mut buffer = vec![0u8; limits.copy_buffer_bytes];

    while !iend_seen {
        if is_cancelled() {
            return Err(ImportError::new(ImportErrorCode::Cancelled));
        }
        chunk_count = chunk_count
            .checked_add(1)
            .ok_or_else(|| ImportError::new(ImportErrorCode::PngMalformed))?;
        if chunk_count > limits.max_png_chunks {
            return Err(ImportError::new(ImportErrorCode::PngMalformed));
        }

        let mut header = [0u8; 8];
        read_exact(&mut reader, &mut header)?;
        let length = u64::from(u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?,
        ));
        if length > limits.max_png_chunk_bytes {
            return Err(ImportError::new(ImportErrorCode::PngMalformed));
        }
        let chunk_type: [u8; 4] = header[4..]
            .try_into()
            .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
        if !chunk_type.iter().all(u8::is_ascii_alphabetic) || chunk_type[2] & 0x20 != 0 {
            return Err(ImportError::new(ImportErrorCode::PngMalformed));
        }
        let ancillary = chunk_type[0] & 0x20 != 0;
        if ancillary {
            metadata_bytes = metadata_bytes
                .checked_add(length)
                .ok_or_else(|| ImportError::new(ImportErrorCode::PngMetadataLimit))?;
            if metadata_bytes > limits.max_png_metadata_bytes {
                return Err(ImportError::new(ImportErrorCode::PngMetadataLimit));
            }
        }
        if matches!(&chunk_type, b"acTL" | b"fcTL" | b"fdAT") {
            return Err(ImportError::new(ImportErrorCode::PngAnimationUnsupported));
        }
        if matches!(&chunk_type, b"zTXt" | b"iCCP") {
            return Err(ImportError::new(ImportErrorCode::PngMetadataLimit));
        }

        let mut crc = Crc32::new();
        crc.update(&chunk_type);
        let mut sha = Sha256::new();
        let mut prefix = Vec::with_capacity(96);
        let mut remaining = length;
        while remaining > 0 {
            if is_cancelled() {
                return Err(ImportError::new(ImportErrorCode::Cancelled));
            }
            let take = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
            read_exact(&mut reader, &mut buffer[..take])?;
            crc.update(&buffer[..take]);
            sha.update(&buffer[..take]);
            let prefix_take = (96usize.saturating_sub(prefix.len())).min(take);
            prefix.extend_from_slice(&buffer[..prefix_take]);
            remaining -= take as u64;
        }
        let mut expected_crc = [0u8; 4];
        read_exact(&mut reader, &mut expected_crc)?;
        if crc.finalize() != u32::from_be_bytes(expected_crc) {
            return Err(ImportError::new(ImportErrorCode::PngMalformed));
        }

        match &chunk_type {
            b"IHDR" => {
                if chunk_count != 1 || ihdr.is_some() || length != 13 || prefix.len() != 13 {
                    return Err(ImportError::new(ImportErrorCode::PngMalformed));
                }
                let width = u32::from_be_bytes(
                    prefix[0..4]
                        .try_into()
                        .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?,
                );
                let height = u32::from_be_bytes(
                    prefix[4..8]
                        .try_into()
                        .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?,
                );
                let pixels = u64::from(width)
                    .checked_mul(u64::from(height))
                    .ok_or_else(|| ImportError::new(ImportErrorCode::PngDimensionLimit))?;
                if width == 0
                    || height == 0
                    || width > limits.max_png_width
                    || height > limits.max_png_height
                    || pixels > limits.max_png_pixels
                {
                    return Err(ImportError::new(ImportErrorCode::PngDimensionLimit));
                }
                ihdr = Some(PngInfo { width, height });
            }
            b"PLTE" => {
                if ihdr.is_none()
                    || idat_seen
                    || plte_seen
                    || length == 0
                    || length > 768
                    || length % 3 != 0
                {
                    return Err(ImportError::new(ImportErrorCode::PngMalformed));
                }
                plte_seen = true;
            }
            b"IDAT" => {
                if ihdr.is_none() || idat_ended {
                    return Err(ImportError::new(ImportErrorCode::PngMalformed));
                }
                idat_seen = true;
            }
            b"IEND" => {
                if length != 0 || ihdr.is_none() || !idat_seen {
                    return Err(ImportError::new(ImportErrorCode::PngMalformed));
                }
                iend_seen = true;
            }
            b"tEXt" => {
                validate_text_keyword(&prefix, length)?;
                let keyword_end = prefix
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| ImportError::new(ImportErrorCode::PngMalformed))?;
                let keyword = String::from_utf8_lossy(&prefix[..keyword_end]);
                if matches!(keyword.as_ref(), "chara" | "ccv3") {
                    card_metadata.push(AcceptedMetadata {
                        logical_path: format!("{logical_path}#tEXt:{keyword}"),
                        sha256: hex_digest(sha.finalize().as_slice()),
                        bytes: length,
                    });
                }
            }
            b"iTXt" => {
                validate_itxt_prefix(&prefix, length)?;
                let keyword_end = prefix
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| ImportError::new(ImportErrorCode::PngMalformed))?;
                let keyword = String::from_utf8_lossy(&prefix[..keyword_end]);
                if matches!(keyword.as_ref(), "chara" | "ccv3") {
                    card_metadata.push(AcceptedMetadata {
                        logical_path: format!("{logical_path}#iTXt:{keyword}"),
                        sha256: hex_digest(sha.finalize().as_slice()),
                        bytes: length,
                    });
                }
            }
            _ if !ancillary => {
                return Err(ImportError::new(ImportErrorCode::PngMalformed));
            }
            _ => {}
        }
        if idat_seen && &chunk_type != b"IDAT" {
            idat_ended = true;
        }
    }

    let mut trailing = [0u8; 1];
    if reader
        .read(&mut trailing)
        .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?
        != 0
    {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    }
    let info = ihdr.ok_or_else(|| ImportError::new(ImportErrorCode::PngMalformed))?;
    decode_png(path, info, limits)?;
    Ok((info, card_metadata))
}

fn decode_png(path: &Path, expected: PngInfo, limits: &ImportLimits) -> Result<()> {
    let file = File::open(path).map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_limits(png::Limits {
        bytes: limits.max_png_decode_bytes,
    });
    decoder.ignore_checksums(false);
    let mut reader = decoder
        .read_info()
        .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
    if reader.info().width != expected.width || reader.info().height != expected.height {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    }
    let output_bytes = reader
        .output_buffer_size()
        .ok_or_else(|| ImportError::new(ImportErrorCode::PngDimensionLimit))?;
    if output_bytes > limits.max_png_decode_bytes {
        return Err(ImportError::new(ImportErrorCode::PngDimensionLimit));
    }
    let mut output = vec![0u8; output_bytes];
    reader
        .next_frame(&mut output)
        .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
    reader
        .finish()
        .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))?;
    Ok(())
}

fn validate_text_keyword(prefix: &[u8], length: u64) -> Result<()> {
    let Some(separator) = prefix.iter().position(|byte| *byte == 0) else {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    };
    if separator == 0 || separator > 79 || separator as u64 >= length {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    }
    Ok(())
}

fn validate_itxt_prefix(prefix: &[u8], length: u64) -> Result<()> {
    let Some(keyword_end) = prefix.iter().position(|byte| *byte == 0) else {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    };
    if keyword_end == 0 || keyword_end > 79 || keyword_end + 3 > prefix.len() {
        return Err(ImportError::new(ImportErrorCode::PngMalformed));
    }
    let compression_flag = prefix[keyword_end + 1];
    let compression_method = prefix[keyword_end + 2];
    if compression_flag != 0 || compression_method != 0 || keyword_end as u64 >= length {
        return Err(ImportError::new(ImportErrorCode::PngMetadataLimit));
    }
    Ok(())
}

fn read_exact(reader: &mut impl Read, buffer: &mut [u8]) -> Result<()> {
    reader
        .read_exact(buffer)
        .map_err(|_| ImportError::new(ImportErrorCode::PngMalformed))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}
