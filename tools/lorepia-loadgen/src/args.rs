use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use crate::{
    assets::AssetOptions,
    bench::BenchOptions,
    db::{BranchProfile, DbOptions},
    stream::{AckProfile, StreamOptions},
    util::{Result, invalid, parse_byte_size, parse_fraction, parse_u64, parse_usize},
    verify::VerifyOptions,
};

pub const HELP: &str = r#"lorepia-loadgen - deterministic non-production evidence tooling

USAGE:
  lorepia-loadgen db --messages N --size BYTES --branch-profile linear|comb|fanout --seed N --output PATH [--receipt PATH]
  lorepia-loadgen assets --count N --total BYTES --duplicate-rate 0..1 --seed N --output DIR [--receipt PATH]
  lorepia-loadgen stream --requests N --ack-profile immediate|delayed|never --seed N --output PATH [--receipt PATH]
  lorepia-loadgen verify --db PATH [--objects DIR] [--full] [--output PATH]
  lorepia-loadgen bench --db PATH --seed N [--objects DIR] [--warmup N] [--iterations N] --output PATH

BYTES accepts B, KiB, MiB, GiB, TiB and decimal KB, MB, GB, TB suffixes.
All output paths use exclusive creation and are never overwritten.
The stream command emits MODEL/SCHEDULE artifacts; it is not Tauri runtime evidence.
"#;

#[derive(Debug)]
pub struct ParsedArgs {
    pub command: Command,
}

#[derive(Debug)]
pub enum Command {
    Db(DbOptions),
    Assets(AssetOptions),
    Stream(StreamOptions),
    Verify(VerifyOptions),
    Bench(BenchOptions),
    Help,
}

impl ParsedArgs {
    pub fn parse<I>(values: I) -> Result<Self>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut values = values.into_iter();
        let Some(command) = values.next() else {
            return Ok(Self {
                command: Command::Help,
            });
        };
        let command = command
            .to_str()
            .ok_or_else(|| invalid("command must be valid UTF-8"))?;
        if matches!(command, "help" | "--help" | "-h") {
            return Ok(Self {
                command: Command::Help,
            });
        }
        let mut flags = FlagMap::parse(values)?;
        let command = match command {
            "db" => Command::Db(DbOptions {
                messages: flags.required_u64("--messages")?,
                target_text_bytes: flags.required_bytes("--size")?,
                branch_profile: flags
                    .required_string("--branch-profile")?
                    .parse::<BranchProfile>()?,
                seed: flags.required_u64("--seed")?,
                output: flags.required_path("--output")?,
                receipt: flags.optional_path("--receipt")?,
            }),
            "assets" => Command::Assets(AssetOptions {
                count: flags.required_u64("--count")?,
                target_active_bytes: flags.required_bytes("--total")?,
                duplicate_rate: flags.required_fraction("--duplicate-rate")?,
                seed: flags.required_u64("--seed")?,
                output: flags.required_path("--output")?,
                receipt: flags.optional_path("--receipt")?,
            }),
            "stream" => Command::Stream(StreamOptions {
                requests: flags.required_u64("--requests")?,
                ack_profile: flags
                    .required_string("--ack-profile")?
                    .parse::<AckProfile>()?,
                seed: flags.required_u64("--seed")?,
                output: flags.required_path("--output")?,
                receipt: flags.optional_path("--receipt")?,
            }),
            "verify" => Command::Verify(VerifyOptions {
                database: flags.required_path("--db")?,
                objects: flags.optional_path("--objects")?,
                full: flags.take_switch("--full")?,
                output: flags.optional_path("--output")?,
            }),
            "bench" => Command::Bench(BenchOptions {
                database: flags.required_path("--db")?,
                objects: flags.optional_path("--objects")?,
                seed: flags.required_u64("--seed")?,
                warmup: flags.optional_usize("--warmup")?.unwrap_or(3),
                iterations: flags.optional_usize("--iterations")?.unwrap_or(30),
                output: flags.required_path("--output")?,
            }),
            _ => return Err(invalid(format!("unknown command: {command}"))),
        };
        flags.finish()?;
        Ok(Self { command })
    }
}

#[derive(Debug)]
struct FlagMap {
    values: BTreeMap<String, Option<OsString>>,
}

impl FlagMap {
    fn parse<I>(values: I) -> Result<Self>
    where
        I: IntoIterator<Item = OsString>,
    {
        let values = values.into_iter().collect::<Vec<_>>();
        let mut parsed = BTreeMap::new();
        let mut index = 0;
        while index < values.len() {
            let flag = values[index]
                .to_str()
                .ok_or_else(|| invalid("flag names must be valid UTF-8"))?;
            if !flag.starts_with("--") || flag.len() == 2 {
                return Err(invalid(format!("unexpected positional argument: {flag}")));
            }
            if parsed.contains_key(flag) {
                return Err(invalid(format!("duplicate flag: {flag}")));
            }
            if matches!(flag, "--full") {
                parsed.insert(flag.to_owned(), None);
                index += 1;
                continue;
            }
            let value = values
                .get(index + 1)
                .ok_or_else(|| invalid(format!("missing value for {flag}")))?;
            if value.to_str().is_some_and(|value| value.starts_with("--")) {
                return Err(invalid(format!("missing value for {flag}")));
            }
            parsed.insert(flag.to_owned(), Some(value.clone()));
            index += 2;
        }
        Ok(Self { values: parsed })
    }

    fn take(&mut self, name: &str) -> Result<OsString> {
        self.values
            .remove(name)
            .ok_or_else(|| invalid(format!("missing required flag {name}")))?
            .ok_or_else(|| invalid(format!("{name} requires a value")))
    }

    fn optional(&mut self, name: &str) -> Result<Option<OsString>> {
        self.values
            .remove(name)
            .map(|value| value.ok_or_else(|| invalid(format!("{name} requires a value"))))
            .transpose()
    }

    fn required_u64(&mut self, name: &str) -> Result<u64> {
        parse_u64(name, &self.take(name)?)
    }

    fn required_bytes(&mut self, name: &str) -> Result<u64> {
        parse_byte_size(name, &self.take(name)?)
    }

    fn required_fraction(&mut self, name: &str) -> Result<f64> {
        parse_fraction(name, &self.take(name)?)
    }

    fn required_string(&mut self, name: &str) -> Result<String> {
        self.take(name)?
            .into_string()
            .map_err(|_| invalid(format!("{name} must be valid UTF-8")))
    }

    fn required_path(&mut self, name: &str) -> Result<PathBuf> {
        Ok(PathBuf::from(self.take(name)?))
    }

    fn optional_path(&mut self, name: &str) -> Result<Option<PathBuf>> {
        Ok(self.optional(name)?.map(PathBuf::from))
    }

    fn optional_usize(&mut self, name: &str) -> Result<Option<usize>> {
        self.optional(name)?
            .map(|value| parse_usize(name, &value))
            .transpose()
    }

    fn take_switch(&mut self, name: &str) -> Result<bool> {
        match self.values.remove(name) {
            Some(None) => Ok(true),
            Some(Some(_)) => Err(invalid(format!("{name} does not accept a value"))),
            None => Ok(false),
        }
    }

    fn finish(self) -> Result<()> {
        if self.values.is_empty() {
            return Ok(());
        }
        Err(invalid(format!(
            "unknown flag(s): {}",
            self.values.keys().cloned().collect::<Vec<_>>().join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<ParsedArgs> {
        ParsedArgs::parse(values.iter().map(OsString::from))
    }

    #[test]
    fn parses_required_db_shape() {
        let parsed = parse(&[
            "db",
            "--messages",
            "1000000",
            "--size",
            "10GiB",
            "--branch-profile",
            "comb",
            "--seed",
            "42",
            "--output",
            "load.sqlite3",
        ])
        .unwrap();
        let Command::Db(options) = parsed.command else {
            panic!("wrong command");
        };
        assert_eq!(options.messages, 1_000_000);
        assert_eq!(options.target_text_bytes, 10 * 1024_u64.pow(3));
        assert_eq!(options.branch_profile, BranchProfile::Comb);
    }

    #[test]
    fn rejects_duplicates_unknowns_and_missing_values() {
        assert!(parse(&["verify", "--db", "a", "--db", "b"]).is_err());
        assert!(parse(&["verify", "--db"]).is_err());
        assert!(parse(&["verify", "--db", "a", "--mystery", "b"]).is_err());
    }
}
