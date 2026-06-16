use std::path::Path;

use serde::Deserialize;

/// An explicit rename hint: tells the diff engine that `from` in the old layout
/// is the same logical field as `to` in the new layout, even when automatic
/// detection would fail (ambiguous swap, rename+reorder, etc.).
///
/// The diff engine only honours the hint when the two fields share the same
/// type; if types differ the hint is silently ignored and the pair is treated
/// as Remove + Add per the spec.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RenameHint {
    pub account: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
struct HintFile {
    #[serde(default)]
    renames: Vec<RenameHint>,
}

/// Load hints from a JSON file.
///
/// Format:
/// ```json
/// { "renames": [{ "account": "Vault", "from": "amount", "to": "balance" }] }
/// ```
pub fn load_hints(path: &Path) -> Result<Vec<RenameHint>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read hints file '{}': {e}", path.display()))?;
    let hf: HintFile = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse hints file '{}': {e}", path.display()))?;
    Ok(hf.renames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{content}").unwrap();
        f
    }

    #[test]
    fn load_single_rename_hint() {
        let f = write_temp(r#"{"renames":[{"account":"Vault","from":"amount","to":"balance"}]}"#);
        let hints = load_hints(f.path()).unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].account, "Vault");
        assert_eq!(hints[0].from, "amount");
        assert_eq!(hints[0].to, "balance");
    }

    #[test]
    fn load_multiple_hints() {
        let f = write_temp(r#"{
            "renames": [
                {"account":"Vault","from":"a","to":"x"},
                {"account":"Market","from":"b","to":"y"}
            ]
        }"#);
        let hints = load_hints(f.path()).unwrap();
        assert_eq!(hints.len(), 2);
    }

    #[test]
    fn empty_renames_array_is_ok() {
        let f = write_temp(r#"{"renames":[]}"#);
        let hints = load_hints(f.path()).unwrap();
        assert!(hints.is_empty());
    }

    #[test]
    fn missing_renames_key_defaults_to_empty() {
        let f = write_temp(r#"{}"#);
        let hints = load_hints(f.path()).unwrap();
        assert!(hints.is_empty());
    }

    #[test]
    fn bad_json_returns_error() {
        let f = write_temp(r#"not json"#);
        assert!(load_hints(f.path()).is_err());
    }

    #[test]
    fn missing_file_returns_error() {
        let err = load_hints(Path::new("/nonexistent/hints.json")).unwrap_err();
        assert!(err.contains("/nonexistent/hints.json"));
    }
}
