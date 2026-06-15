use std::path::Path;

use layoutd_core::classify::{ClassifiedChange, Safety};
use layoutd_core::diff::ChangeKind;
use serde_json::Value;

/// A single danger acknowledgement from a layoutd.ack file.
#[derive(Debug, Clone)]
pub struct Ack {
    pub account: String,
    pub field: String,
    pub change: String,
    pub note: String,
}

impl Ack {
    fn matches(&self, account: &str, c: &ClassifiedChange) -> bool {
        if self.account != account {
            return false;
        }
        if self.field != c.change.name {
            return false;
        }
        let kind_str = change_kind_slug(&c.change.kind);
        self.change == kind_str || self.change == "any"
    }
}

fn change_kind_slug(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Removed { .. }              => "removed",
        ChangeKind::TypeChanged { .. }          => "type_changed",
        ChangeKind::TypeChangedAndReordered { .. } => "type_changed_and_reordered",
        ChangeKind::Reordered { .. }            => "reordered",
        ChangeKind::Added { .. }                => "added",
        ChangeKind::Renamed { .. }              => "renamed",
        ChangeKind::Unchanged                   => "unchanged",
    }
}

/// Load acknowledgements from a JSON file.
///
/// Format:
/// ```json
/// {
///   "acknowledged": [
///     { "account": "Vault", "field": "balance", "change": "removed", "note": "migrated" }
///   ]
/// }
/// ```
/// `change` can be a `change_kind_slug` string or `"any"` to match any change kind.
pub fn load(path: &Path) -> Result<Vec<Ack>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read ack file '{}': {e}", path.display()))?;

    let value: Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse ack file '{}': {e}", path.display()))?;

    let arr = value
        .get("acknowledged")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            format!("ack file '{}' must have an 'acknowledged' array", path.display())
        })?;

    let mut acks = Vec::new();
    for (i, entry) in arr.iter().enumerate() {
        let account = str_field(entry, "account", i)?;
        let field   = str_field(entry, "field",   i)?;
        let change  = str_field(entry, "change",  i)?;
        let note    = entry.get("note").and_then(|v| v.as_str()).unwrap_or("").to_string();
        acks.push(Ack { account, field, change, note });
    }

    Ok(acks)
}

fn str_field(entry: &Value, key: &str, idx: usize) -> Result<String, String> {
    entry
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("ack entry #{idx} missing required string field '{key}'"))
}

/// Result of matching dangers against acknowledgements.
pub struct AckResult {
    /// Danger changes that have no matching acknowledgement.
    pub unacknowledged: Vec<usize>,
    /// Acknowledged changes (index into `classified`).
    pub acknowledged: Vec<usize>,
    /// Acks in the file that did not match any actual danger change.
    pub stale: Vec<Ack>,
}

/// Match a set of classified changes against the loaded acknowledgements.
pub fn check(account: &str, classified: &[ClassifiedChange], acks: &[Ack]) -> AckResult {
    let dangers: Vec<usize> = classified
        .iter()
        .enumerate()
        .filter(|(_, c)| c.safety == Safety::Danger)
        .map(|(i, _)| i)
        .collect();

    let mut acknowledged = Vec::new();
    let mut unacknowledged = Vec::new();
    let mut used_ack: Vec<bool> = vec![false; acks.len()];

    'outer: for &di in &dangers {
        let c = &classified[di];
        for (ai, ack) in acks.iter().enumerate() {
            if ack.matches(account, c) {
                acknowledged.push(di);
                used_ack[ai] = true;
                continue 'outer;
            }
        }
        unacknowledged.push(di);
    }

    let stale: Vec<Ack> = acks
        .iter()
        .zip(used_ack.iter())
        .filter(|(_, used)| !**used)
        .map(|(a, _)| a.clone())
        .collect();

    AckResult { unacknowledged, acknowledged, stale }
}
