use crate::{AssetError, AssetLimits, AssetMime, Result};

pub(crate) const PROBE_BYTES: usize = 64 * 1024;
pub(crate) const TAIL_BYTES: usize = 16;

pub(crate) fn validate(
    declared: AssetMime,
    prefix: &[u8],
    tail: &[u8],
    size: u64,
    limits: &AssetLimits,
) -> Result<AssetMime> {
    let detected = detect(prefix, tail, size, limits)?;
    if detected != declared {
        return Err(AssetError::MimeMismatch {
            declared: declared.to_string(),
            detected: detected.to_string(),
        });
    }
    Ok(detected)
}

fn detect(prefix: &[u8], tail: &[u8], size: u64, limits: &AssetLimits) -> Result<AssetMime> {
    if let Some((width, height)) = png_dimensions(prefix) {
        validate_dimensions(width, height, limits)?;
        return Ok(AssetMime::Png);
    }
    if let Some((width, height)) = gif_dimensions(prefix, tail) {
        validate_dimensions(width, height, limits)?;
        return Ok(AssetMime::Gif);
    }
    if let Some((width, height)) = webp_dimensions(prefix, size) {
        if let (Some(width), Some(height)) = (width, height) {
            validate_dimensions(width, height, limits)?;
        }
        return Ok(AssetMime::WebP);
    }
    if let Some((width, height)) = jpeg_dimensions(prefix, tail) {
        validate_dimensions(width, height, limits)?;
        return Ok(AssetMime::Jpeg);
    }
    if valid_wav(prefix, size) {
        return Ok(AssetMime::Wav);
    }
    if valid_mp3(prefix) {
        return Ok(AssetMime::Mp3);
    }
    if valid_ogg(prefix) {
        return Ok(AssetMime::Ogg);
    }
    if prefix.starts_with(b"fLaC") && size >= 42 {
        return Ok(AssetMime::Flac);
    }
    Err(AssetError::UnsupportedContent)
}

fn validate_dimensions(width: u32, height: u32, limits: &AssetLimits) -> Result<()> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > limits.max_image_width
        || height > limits.max_image_height
        || pixels > limits.max_image_pixels
    {
        return Err(AssetError::InvalidInput {
            field: "image dimensions",
            reason: "image dimensions exceed configured decode-safety limits",
        });
    }
    Ok(())
}

fn png_dimensions(prefix: &[u8]) -> Option<(u32, u32)> {
    if prefix.len() < 33
        || prefix.get(..8)? != b"\x89PNG\r\n\x1a\n"
        || prefix.get(8..12)? != [0, 0, 0, 13]
        || prefix.get(12..16)? != b"IHDR"
    {
        return None;
    }
    let width = u32::from_be_bytes(prefix.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(prefix.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

fn gif_dimensions(prefix: &[u8], tail: &[u8]) -> Option<(u32, u32)> {
    if prefix.len() < 13
        || !(prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a"))
        || tail.last().copied() != Some(0x3b)
    {
        return None;
    }
    let width = u16::from_le_bytes(prefix.get(6..8)?.try_into().ok()?);
    let height = u16::from_le_bytes(prefix.get(8..10)?.try_into().ok()?);
    Some((u32::from(width), u32::from(height)))
}

fn webp_dimensions(prefix: &[u8], size: u64) -> Option<(Option<u32>, Option<u32>)> {
    if prefix.len() < 16 || prefix.get(..4)? != b"RIFF" || prefix.get(8..12)? != b"WEBP" {
        return None;
    }
    let declared = u32::from_le_bytes(prefix.get(4..8)?.try_into().ok()?);
    if u64::from(declared).checked_add(8)? != size {
        return None;
    }
    match prefix.get(12..16)? {
        b"VP8X" if prefix.len() >= 30 => {
            let width = 1 + read_u24_le(prefix.get(24..27)?)?;
            let height = 1 + read_u24_le(prefix.get(27..30)?)?;
            Some((Some(width), Some(height)))
        }
        b"VP8L" if prefix.len() >= 25 && prefix[20] == 0x2f => {
            let bits = u32::from_le_bytes(prefix.get(21..25)?.try_into().ok()?);
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            Some((Some(width), Some(height)))
        }
        b"VP8 " if prefix.len() >= 30 => {
            let marker = prefix.get(23..26)?;
            if marker != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let width = u16::from_le_bytes(prefix.get(26..28)?.try_into().ok()?) & 0x3fff;
            let height = u16::from_le_bytes(prefix.get(28..30)?.try_into().ok()?) & 0x3fff;
            Some((Some(u32::from(width)), Some(u32::from(height))))
        }
        _ => None,
    }
}

fn read_u24_le(bytes: &[u8]) -> Option<u32> {
    Some(
        u32::from(bytes.first().copied()?)
            | (u32::from(bytes.get(1).copied()?) << 8)
            | (u32::from(bytes.get(2).copied()?) << 16),
    )
}

fn jpeg_dimensions(prefix: &[u8], tail: &[u8]) -> Option<(u32, u32)> {
    if !prefix.starts_with(&[0xff, 0xd8, 0xff]) || !tail.ends_with(&[0xff, 0xd9]) {
        return None;
    }
    let mut cursor = 2usize;
    while cursor + 4 <= prefix.len() {
        while prefix.get(cursor) == Some(&0xff) {
            cursor += 1;
        }
        let marker = *prefix.get(cursor)?;
        cursor += 1;
        if matches!(marker, 0xd8 | 0xd9) {
            continue;
        }
        if marker == 0xda {
            break;
        }
        let length = usize::from(u16::from_be_bytes(
            prefix.get(cursor..cursor + 2)?.try_into().ok()?,
        ));
        if length < 2 || cursor.checked_add(length)? > prefix.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) && length >= 7
        {
            let height = u16::from_be_bytes(prefix.get(cursor + 3..cursor + 5)?.try_into().ok()?);
            let width = u16::from_be_bytes(prefix.get(cursor + 5..cursor + 7)?.try_into().ok()?);
            return Some((u32::from(width), u32::from(height)));
        }
        cursor += length;
    }
    None
}

fn valid_wav(prefix: &[u8], size: u64) -> bool {
    if prefix.len() < 44 || !prefix.starts_with(b"RIFF") || prefix.get(8..12) != Some(b"WAVE") {
        return false;
    }
    let Ok(declared_bytes) = <[u8; 4]>::try_from(&prefix[4..8]) else {
        return false;
    };
    let declared = u32::from_le_bytes(declared_bytes);
    u64::from(declared).checked_add(8) == Some(size)
        && prefix.windows(4).any(|window| window == b"fmt ")
        && prefix.windows(4).any(|window| window == b"data")
}

fn valid_mp3(prefix: &[u8]) -> bool {
    let offset = if prefix.starts_with(b"ID3") {
        if prefix.len() < 10 || prefix[6..10].iter().any(|byte| byte & 0x80 != 0) {
            return false;
        }
        let size = prefix[6..10]
            .iter()
            .fold(0usize, |value, byte| (value << 7) | usize::from(*byte));
        10usize.saturating_add(size)
    } else {
        0
    };
    prefix
        .get(offset..offset.saturating_add(2))
        .is_some_and(|bytes| bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0 && bytes[1] & 0x06 != 0)
}

fn valid_ogg(prefix: &[u8]) -> bool {
    prefix.len() >= 32
        && prefix.starts_with(b"OggS")
        && prefix[4] == 0
        && (prefix.windows(8).any(|window| window == b"OpusHead")
            || prefix.windows(7).any(|window| window == b"\x01vorbis"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dimension_bombs_before_decode() {
        let mut png = vec![0u8; 33];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[8..12].copy_from_slice(&13u32.to_be_bytes());
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let limits = AssetLimits::new(1_000_000, 1_000_000).expect("limits");
        assert!(matches!(
            validate(AssetMime::Png, &png, &png, png.len() as u64, &limits),
            Err(AssetError::InvalidInput {
                field: "image dimensions",
                ..
            })
        ));
    }

    #[test]
    fn every_allowlisted_family_requires_its_magic_contract() {
        let limits = AssetLimits::new(1_000_000, 1_000_000).expect("limits");

        let mut png = vec![0u8; 33];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[8..12].copy_from_slice(&13u32.to_be_bytes());
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&1u32.to_be_bytes());
        png[20..24].copy_from_slice(&1u32.to_be_bytes());

        let jpeg = vec![
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x02, 0x00, 0x03, 0x01, 0x01, 0x11,
            0x00, 0xff, 0xd9,
        ];

        let mut webp = vec![0u8; 30];
        webp[..4].copy_from_slice(b"RIFF");
        webp[4..8].copy_from_slice(&22u32.to_le_bytes());
        webp[8..12].copy_from_slice(b"WEBP");
        webp[12..16].copy_from_slice(b"VP8X");

        let mut gif = vec![0u8; 14];
        gif[..6].copy_from_slice(b"GIF89a");
        gif[6..8].copy_from_slice(&1u16.to_le_bytes());
        gif[8..10].copy_from_slice(&1u16.to_le_bytes());
        *gif.last_mut().expect("GIF trailer") = 0x3b;

        let mut wav = vec![0u8; 44];
        wav[..4].copy_from_slice(b"RIFF");
        wav[4..8].copy_from_slice(&36u32.to_le_bytes());
        wav[8..12].copy_from_slice(b"WAVE");
        wav[12..16].copy_from_slice(b"fmt ");
        wav[36..40].copy_from_slice(b"data");

        let mp3 = vec![0xff, 0xfb, 0x90, 0x64];
        let mut ogg = vec![0u8; 40];
        ogg[..4].copy_from_slice(b"OggS");
        ogg[24..32].copy_from_slice(b"OpusHead");
        let mut flac = vec![0u8; 42];
        flac[..4].copy_from_slice(b"fLaC");

        for (mime, bytes) in [
            (AssetMime::Png, png),
            (AssetMime::Jpeg, jpeg),
            (AssetMime::WebP, webp),
            (AssetMime::Gif, gif),
            (AssetMime::Wav, wav),
            (AssetMime::Mp3, mp3),
            (AssetMime::Ogg, ogg),
            (AssetMime::Flac, flac),
        ] {
            assert_eq!(
                validate(mime, &bytes, &bytes, bytes.len() as u64, &limits).expect("valid magic"),
                mime
            );
        }
    }
}
