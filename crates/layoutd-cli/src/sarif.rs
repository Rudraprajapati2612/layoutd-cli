use std::path::Path;

use layoutd_core::classify::{ClassifiedChange, Safety};
use layoutd_core::diff::ChangeKind;
use serde_json::{json, Value};

/// Build a SARIF 2.1.0 document from a classified change list.
///
/// `zc` selects zero-copy rule IDs (LD005/LD006) for reorder/mid-insert/same-size-type-change;
/// when `false` the standard Borsh rule IDs (LD001–LD004) are used.
///
/// Safe changes produce no results.
pub fn build(classified: &[ClassifiedChange], new_idl_path: &Path, zc: bool) -> Value {
    let uri = new_idl_path.to_string_lossy();

    let results: Vec<Value> = classified
        .iter()
        .filter(|c| c.safety != Safety::Safe)
        .map(|c| build_result(c, &uri, zc))
        .collect();

    json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "layoutd",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/your-org/layoutd",
                    "rules": rules()
                }
            },
            "results": results
        }]
    })
}

fn build_result(c: &ClassifiedChange, uri: &str, zc: bool) -> Value {
    json!({
        "ruleId": rule_id(c, zc),
        "level": sarif_level(&c.safety),
        "message": {
            "text": format!("Field '{}': {}", c.change.name, c.reason)
        },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": {
                    "uri": uri,
                    "uriBaseId": "%SRCROOT%"
                },
                "region": { "startLine": 1 }
            }
        }]
    })
}

fn sarif_level(safety: &Safety) -> &'static str {
    match safety {
        Safety::Danger => "error",
        Safety::Review => "warning",
        Safety::Safe   => "none",
    }
}

fn rule_id(c: &ClassifiedChange, zc: bool) -> &'static str {
    if zc {
        return match (&c.safety, &c.change.kind) {
            (Safety::Danger, ChangeKind::Removed { .. })                    => "LD001",
            (Safety::Danger, ChangeKind::Reordered { .. })                  => "LD005",
            (Safety::Danger, ChangeKind::Added { .. })                      => "LD005",
            (Safety::Danger, ChangeKind::TypeChangedAndReordered { .. })    => "LD005",
            (Safety::Danger, _)                                             => "LD002",
            (Safety::Review, ChangeKind::Added { .. })                      => "LD004",
            (Safety::Review, _)                                             => "LD006",
            (Safety::Safe,   _)                                             => "LD000",
        };
    }

    match (&c.safety, &c.change.kind) {
        (Safety::Danger, ChangeKind::Removed { .. }) => "LD001",
        (Safety::Danger, _)                          => "LD002",
        (Safety::Review, ChangeKind::Added { .. })   => "LD004",
        (Safety::Review, _)                          => "LD003",
        (Safety::Safe,   _)                          => "LD000",
    }
}

fn rules() -> Value {
    json!([
        {
            "id": "LD001",
            "name": "FieldRemoved",
            "shortDescription": { "text": "Field removed from account struct" },
            "fullDescription": {
                "text": "Removing a field causes permanent data loss and shifts byte offsets for all \
                         following fields, corrupting reads through the new layout on existing accounts."
            },
            "defaultConfiguration": { "level": "error" },
            "properties": { "tags": ["correctness", "solana", "migration"] }
        },
        {
            "id": "LD002",
            "name": "TypeDanger",
            "shortDescription": { "text": "Unsafe type change" },
            "fullDescription": {
                "text": "The type change cannot be proven safe: narrowing (overflow risk), sign flip \
                         (same bits, different meaning), float/integer reinterpretation, or \
                         incompatible encoding (Vec, String)."
            },
            "defaultConfiguration": { "level": "error" },
            "properties": { "tags": ["correctness", "solana", "migration"] }
        },
        {
            "id": "LD003",
            "name": "TypeWidened",
            "shortDescription": { "text": "Integer or float widened" },
            "fullDescription": {
                "text": "The field type was widened to a larger compatible type. The value fits, \
                         but alignment implications and signedness assumptions should be verified."
            },
            "defaultConfiguration": { "level": "warning" },
            "properties": { "tags": ["correctness", "solana", "migration"] }
        },
        {
            "id": "LD004",
            "name": "FieldAddedMiddle",
            "shortDescription": { "text": "Field inserted in middle of struct" },
            "fullDescription": {
                "text": "A field was inserted before the last position. Safe for Borsh accounts \
                         (serialization matches by name), but verify padding and alignment for \
                         zero-copy accounts."
            },
            "defaultConfiguration": { "level": "warning" },
            "properties": { "tags": ["correctness", "solana", "migration"] }
        },
        {
            "id": "LD005",
            "name": "ZcOffsetShift",
            "shortDescription": { "text": "Zero-copy layout change shifts byte offsets" },
            "fullDescription": {
                "text": "In repr(C) zero-copy structs, field declaration order determines byte offsets. \
                         Reordering fields or inserting a field in the middle shifts the byte address \
                         of every following field, causing silent data corruption on existing accounts."
            },
            "defaultConfiguration": { "level": "error" },
            "properties": { "tags": ["correctness", "solana", "zero-copy", "migration"] }
        },
        {
            "id": "LD006",
            "name": "ZcTypeReinterpret",
            "shortDescription": { "text": "Same-size type change in zero-copy struct" },
            "fullDescription": {
                "text": "The field type changed to one with the same size and alignment. \
                         The byte offset is preserved but the bit interpretation changes — \
                         verify semantic correctness before shipping."
            },
            "defaultConfiguration": { "level": "warning" },
            "properties": { "tags": ["correctness", "solana", "zero-copy", "migration"] }
        }
    ])
}
