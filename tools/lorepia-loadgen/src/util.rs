use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Component, Path, PathBuf},
};

use serde::Serialize;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub const MIB: u64 = 1024 * 1024;

pub fn invalid(message: impl Into<String>) -> Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()).into()
}

pub fn parse_u64(name: &str, value: &OsStr) -> Result<u64> {
    let value = value
        .to_str()
        .ok_or_else(|| invalid(format!("{name} must be valid UTF-8")))?;
    value
        .parse::<u64>()
        .map_err(|_| invalid(format!("{name} must be an unsigned integer")))
}

pub fn parse_usize(name: &str, value: &OsStr) -> Result<usize> {
    let parsed = parse_u64(name, value)?;
    usize::try_from(parsed).map_err(|_| invalid(format!("{name} exceeds this platform's range")))
}

pub fn parse_byte_size(name: &str, value: &OsStr) -> Result<u64> {
    let raw = value
        .to_str()
        .ok_or_else(|| invalid(format!("{name} must be valid UTF-8")))?
        .trim();
    if raw.is_empty() {
        return Err(invalid(format!("{name} must not be empty")));
    }
    let digit_end = raw
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(raw.len());
    if digit_end == 0 {
        return Err(invalid(format!(
            "{name} must begin with an unsigned integer"
        )));
    }
    let number = raw[..digit_end]
        .parse::<u64>()
        .map_err(|_| invalid(format!("{name} is outside the supported range")))?;
    let suffix = raw[digit_end..].to_ascii_lowercase();
    let multiplier = match suffix.as_str() {
        "" | "b" => 1,
        "kib" => 1024,
        "mib" => 1024_u64.pow(2),
        "gib" => 1024_u64.pow(3),
        "tib" => 1024_u64.pow(4),
        "kb" => 1000,
        "mb" => 1000_u64.pow(2),
        "gb" => 1000_u64.pow(3),
        "tb" => 1000_u64.pow(4),
        _ => {
            return Err(invalid(format!(
                "{name} has an unsupported suffix (use B, KiB, MiB, GiB, TiB, KB, MB, GB, or TB)"
            )));
        }
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| invalid(format!("{name} overflows 64-bit bytes")))
}

pub fn parse_fraction(name: &str, value: &OsStr) -> Result<f64> {
    let value = value
        .to_str()
        .ok_or_else(|| invalid(format!("{name} must be valid UTF-8")))?;
    let parsed = value
        .parse::<f64>()
        .map_err(|_| invalid(format!("{name} must be a number between 0 and 1")))?;
    if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
        return Err(invalid(format!("{name} must be between 0 and 1")));
    }
    Ok(parsed)
}

pub fn canonical_existing_file(path: &Path) -> Result<PathBuf> {
    reject_unsafe_input(path)?;
    let canonical = fs::canonicalize(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !canonical.is_file() {
        return Err(invalid(format!(
            "input must be a regular non-symlink file: {}",
            path.display()
        )));
    }
    Ok(canonical)
}

pub fn canonical_existing_dir(path: &Path) -> Result<PathBuf> {
    reject_unsafe_input(path)?;
    let canonical = fs::canonicalize(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !canonical.is_dir() {
        return Err(invalid(format!(
            "input must be a non-symlink directory: {}",
            path.display()
        )));
    }
    Ok(canonical)
}

pub fn prepare_new_file(path: &Path) -> Result<(PathBuf, File)> {
    let output = resolve_new_output(path)?;
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&output)?;
    Ok((output, file))
}

pub fn prepare_new_directory(path: &Path) -> Result<PathBuf> {
    let output = resolve_new_output(path)?;
    fs::create_dir(&output)?;
    Ok(output)
}

fn resolve_new_output(path: &Path) -> Result<PathBuf> {
    reject_unsafe_output(path)?;
    if fs::symlink_metadata(path).is_ok() {
        return Err(invalid(format!(
            "refusing to overwrite existing output: {}",
            path.display()
        )));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| invalid("output must name a file or directory"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|error| {
        invalid(format!(
            "output parent must already exist and be accessible ({}): {error}",
            parent.display()
        ))
    })?;
    Ok(parent.join(file_name))
}

fn reject_unsafe_input(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path == Path::new("/") {
        return Err(invalid("path must not be empty or the filesystem root"));
    }
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(invalid("path must not contain '..' components"));
    }
    Ok(())
}

fn reject_unsafe_output(path: &Path) -> Result<()> {
    reject_unsafe_input(path)?;
    if path.file_name().is_none() {
        return Err(invalid("output must have a final path component"));
    }
    Ok(())
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let (_, file) = prepare_new_file(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

pub fn emit_receipt<T: Serialize>(output: Option<&Path>, receipt: &T) -> Result<()> {
    if let Some(output) = output {
        write_json_atomic(output, receipt)
    } else {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        serde_json::to_writer_pretty(&mut lock, receipt)?;
        lock.write_all(b"\n")?;
        Ok(())
    }
}

pub fn ensure_free_space(parent_or_file: &Path, required: u64) -> Result<u64> {
    let probe = if parent_or_file.is_dir() {
        parent_or_file
    } else {
        parent_or_file.parent().unwrap_or_else(|| Path::new("."))
    };
    let available = fs2::available_space(probe)?;
    if available < required {
        return Err(invalid(format!(
            "free-space preflight failed: required {required} bytes, available {available} bytes"
        )));
    }
    Ok(available)
}

pub fn checked_sum_file_sizes(paths: &[PathBuf]) -> Result<u64> {
    paths.iter().try_fold(0_u64, |total, path| {
        if !path.exists() {
            return Ok(total);
        }
        let bytes = fs::metadata(path)?.len();
        total
            .checked_add(bytes)
            .ok_or_else(|| invalid("file size total overflowed"))
    })
}

#[derive(Clone, Debug)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.state = value;
        value.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

pub fn deterministic_id(namespace: u64, seed: u64, index: u64) -> String {
    format!("{:016x}{index:016x}", seed ^ namespace)
}

pub fn sqlite_sidecars(database: &Path) -> Vec<PathBuf> {
    let base = database.as_os_str().to_string_lossy();
    vec![
        database.to_path_buf(),
        PathBuf::from(format!("{base}-wal")),
        PathBuf::from(format!("{base}-shm")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binary_and_decimal_sizes_with_overflow_checks() {
        assert_eq!(
            parse_byte_size("size", OsStr::new("10GiB")).unwrap(),
            10 * 1024_u64.pow(3)
        );
        assert_eq!(
            parse_byte_size("size", OsStr::new("2GB")).unwrap(),
            2_000_000_000
        );
        assert!(parse_byte_size("size", OsStr::new("18446744073709551615TiB")).is_err());
        assert!(parse_byte_size("size", OsStr::new("-1")).is_err());
    }

    #[test]
    fn deterministic_ids_are_fixed_width_and_distinct() {
        let first = deterministic_id(1, 42, 0);
        let second = deterministic_id(1, 42, 1);
        assert_eq!(first.len(), 32);
        assert_ne!(first, second);
        assert_eq!(first, deterministic_id(1, 42, 0));
    }

    #[test]
    fn exclusive_output_creation_refuses_an_existing_target() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("receipt.json");
        write_json_atomic(&output, &serde_json::json!({"first": true})).unwrap();
        assert!(write_json_atomic(&output, &serde_json::json!({"second": true})).is_err());
        let retained: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        assert_eq!(retained["first"], true);
        assert!(retained.get("second").is_none());
    }
}
