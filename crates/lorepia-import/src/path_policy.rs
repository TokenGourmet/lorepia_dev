use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;

use crate::{ImportError, ImportErrorCode, ImportLimits, Result};

#[derive(Clone, Debug)]
pub(crate) struct PortablePath {
    pub logical: String,
    pub collision_key: String,
    pub components: Vec<String>,
    pub is_directory: bool,
}

pub(crate) fn validate_path(
    raw: &[u8],
    directory_hint: bool,
    limits: &ImportLimits,
) -> Result<PortablePath> {
    let raw =
        std::str::from_utf8(raw).map_err(|_| ImportError::new(ImportErrorCode::UnsafePath))?;
    if raw.is_empty()
        || raw.as_bytes().contains(&0)
        || raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.contains('\\')
        || raw.contains(':')
        || raw.len() > limits.max_path_bytes
        || raw.nfc().collect::<String>() != raw
    {
        return Err(ImportError::new(ImportErrorCode::UnsafePath));
    }

    let has_trailing_slash = raw.ends_with('/');
    if has_trailing_slash != directory_hint {
        return Err(ImportError::new(ImportErrorCode::UnsafePath));
    }
    let logical = raw.trim_end_matches('/');
    if logical.is_empty() {
        return Err(ImportError::new(ImportErrorCode::UnsafePath));
    }

    let mut components = Vec::new();
    for component in logical.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > limits.max_component_bytes
            || component.ends_with(['.', ' '])
            || component.chars().any(|character| {
                character.is_control() || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
            })
            || is_windows_device(component)
        {
            return Err(ImportError::new(ImportErrorCode::UnsafePath));
        }
        components.push(component.to_owned());
    }
    if components.len() > limits.max_path_depth {
        return Err(ImportError::new(ImportErrorCode::UnsafePath));
    }

    let collision_key = portable_case_key(logical);
    Ok(PortablePath {
        logical: logical.to_owned(),
        collision_key,
        components,
        is_directory: directory_hint,
    })
}

pub(crate) fn insert_unique(
    path: &PortablePath,
    files: &mut HashSet<String>,
    directories: &mut HashSet<String>,
) -> Result<()> {
    let key = &path.collision_key;
    if path.is_directory {
        if files.contains(key) || !directories.insert(key.clone()) {
            return Err(ImportError::new(ImportErrorCode::DuplicatePath));
        }
    } else {
        if files.contains(key) || directories.contains(key) {
            return Err(ImportError::new(ImportErrorCode::DuplicatePath));
        }
        for depth in 1..path.components.len() {
            let parent = portable_case_key(&path.components[..depth].join("/"));
            if files.contains(&parent) {
                return Err(ImportError::new(ImportErrorCode::DuplicatePath));
            }
            directories.insert(parent);
        }
        let prefix = format!("{key}/");
        if files.iter().any(|existing| existing.starts_with(&prefix))
            || directories
                .iter()
                .any(|existing| existing.starts_with(&prefix))
        {
            return Err(ImportError::new(ImportErrorCode::DuplicatePath));
        }
        files.insert(key.clone());
    }
    Ok(())
}

fn portable_case_key(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .nfc()
        .collect()
}

fn is_windows_device(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let upper = stem
        .chars()
        .map(|character| match character {
            '¹' => '1',
            '²' => '2',
            '³' => '3',
            other => other,
        })
        .collect::<String>()
        .to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        upper.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escape_aliases_and_normalization_ambiguity() {
        let limits = ImportLimits::default();
        for value in [
            b"../x.png".as_slice(),
            b"/x.png",
            b"C:/x.png",
            b"a\\x.png",
            b"CON.txt",
            "cafe\u{301}.png".as_bytes(),
        ] {
            assert!(validate_path(value, false, &limits).is_err());
        }
    }

    #[test]
    fn catches_case_and_prefix_collisions() {
        let limits = ImportLimits::default();
        let mut files = HashSet::new();
        let mut directories = HashSet::new();
        let first = validate_path(b"Assets/Card.png", false, &limits).expect("first");
        insert_unique(&first, &mut files, &mut directories).expect("insert first");
        let alias = validate_path(b"assets/card.PNG", false, &limits).expect("alias");
        assert!(insert_unique(&alias, &mut files, &mut directories).is_err());

        let parent_file = validate_path(b"manifest", false, &limits).expect("parent");
        insert_unique(&parent_file, &mut files, &mut directories).expect("parent insert");
        let child = validate_path(b"manifest/value.json", false, &limits).expect("child");
        assert!(insert_unique(&child, &mut files, &mut directories).is_err());
    }

    #[test]
    fn rejects_portable_windows_device_families() {
        let limits = ImportLimits::default();
        for name in [
            "CON",
            "prn.txt",
            "AUX.json",
            "nul.png",
            "CLOCK$",
            "CONIN$",
            "conout$",
            "COM1.png",
            "com9",
            "LPT1.txt",
            "lpt9",
            "COM¹.png",
            "LPT².json",
            "COM³",
        ] {
            assert!(
                validate_path(name.as_bytes(), false, &limits).is_err(),
                "{name}"
            );
        }
    }
}
