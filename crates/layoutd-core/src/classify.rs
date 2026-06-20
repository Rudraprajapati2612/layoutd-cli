use crate::borsh::{Offset, Size};
use crate::diff::{ChangeKind, FieldChange};
use crate::idl::FieldType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Safety {
    Safe,
    Review,
    Danger,
}

#[derive(Debug, Clone)]
pub struct ClassifiedChange {
    pub change: FieldChange,
    pub safety: Safety,
    pub reason: &'static str,
}

/// Primary API: classify a full change list from the diff engine.
pub fn classify_all(changes: Vec<FieldChange>) -> Vec<ClassifiedChange> {
    // Use the highest new-layout index among fields that survived from the old struct.
    // Removed fields have no new_layout; purely new (Added) fields have no old_layout.
    // This correctly handles removal + reorder combos where max old_index would be misleading.
    let max_old_in_new = changes
        .iter()
        .filter(|c| c.old_layout.is_some() && c.new_layout.is_some())
        .filter_map(|c| c.new_layout.as_ref())
        .map(|fl| fl.index)
        .max();

    // If every old field has a fixed offset and fixed size, the old account is exact-sized.
    // Appending to an exact-sized account is DANGER: old accounts have no bytes for the new field.
    let old_is_fixed = changes
        .iter()
        .filter_map(|c| c.old_layout.as_ref())
        .all(|fl| matches!(fl.offset, Offset::Fixed(_)) && matches!(fl.size, Size::Fixed(_)));

    changes.into_iter().map(|c| classify_one(c, max_old_in_new, old_is_fixed)).collect()
}

/// Classify a single change.
/// `max_old_index` — highest new-layout index among surviving old fields (for append detection).
/// `old_is_fixed`  — true when every old field has a fixed offset+size (exact-sized account).
pub fn classify_one(
    change: FieldChange,
    max_old_index: Option<usize>,
    old_is_fixed: bool,
) -> ClassifiedChange {
    let (safety, reason) = match &change.kind {
        ChangeKind::Unchanged => (Safety::Safe, "field unchanged"),

        ChangeKind::Added { at_index } => {
            let is_appended = max_old_index.map_or(true, |max| *at_index > max);
            if is_appended {
                if old_is_fixed {
                    (
                        Safety::Danger,
                        "field appended — old accounts are exact-sized and lack bytes for this field; realloc required before deserialization",
                    )
                } else {
                    (
                        Safety::Review,
                        "field appended — old accounts may lack bytes; verify realloc",
                    )
                }
            } else {
                (
                    Safety::Review,
                    "field inserted in existing layout — existing accounts need migration",
                )
            }
        }

        ChangeKind::Removed { .. } => (
            Safety::Danger,
            "field removed — permanent data loss; suggest marking deprecated instead",
        ),

        ChangeKind::Reordered { .. } => (
            Safety::Danger,
            "field reordered — Borsh is positional, existing accounts will deserialize incorrectly",
        ),

        // Rename was confirmed by diff engine (same index + same type): value carries over intact.
        ChangeKind::Renamed { .. } => (
            Safety::Safe,
            "field renamed — same type and position, value carries over unchanged",
        ),

        ChangeKind::TypeChanged { old_type, new_type } => {
            classify_type_change(old_type, new_type)
        }

        ChangeKind::TypeChangedAndReordered { old_type, new_type, .. } => {
            classify_type_change(old_type, new_type)
        }
    };

    ClassifiedChange { change, safety, reason }
}

fn classify_type_change(old_ty: &FieldType, new_ty: &FieldType) -> (Safety, &'static str) {
    // Float widen changes byte size (4→8) — existing accounts need migration.
    if matches!((old_ty, new_ty), (FieldType::F32, FieldType::F64)) {
        return (
            Safety::Danger,
            "float widen f32→f64 — byte size doubles, existing accounts require migration",
        );
    }

    if is_safe_widen(old_ty, new_ty) {
        return (
            Safety::Danger,
            "integer widen — byte size expands, existing accounts require migration before use",
        );
    }

    if is_narrowing(old_ty, new_ty) {
        return (
            Safety::Danger,
            "narrowing type change — possible overflow or data truncation",
        );
    }

    if is_sign_flip(old_ty, new_ty) {
        return (
            Safety::Danger,
            "sign flip — same bytes reinterpreted, values will be wrong for large numbers",
        );
    }

    if is_float_int_change(old_ty, new_ty) {
        return (
            Safety::Danger,
            "float/integer reinterpretation — bits mean completely different things",
        );
    }

    if matches!(old_ty, FieldType::String) || matches!(new_ty, FieldType::String) {
        return (
            Safety::Danger,
            "string reinterpretation — variable-length encoding incompatible with fixed types",
        );
    }

    if matches!(old_ty, FieldType::Vec(_)) != matches!(new_ty, FieldType::Vec(_)) {
        return (
            Safety::Danger,
            "vec reinterpretation — length-prefixed encoding incompatible with other types",
        );
    }

    if matches!(old_ty, FieldType::Unknown(_)) || matches!(new_ty, FieldType::Unknown(_)) {
        return (
            Safety::Danger,
            "unknown type involved — cannot reason about byte-level safety",
        );
    }

    (Safety::Danger, "type reinterpretation — bytes now mean something different")
}

fn is_safe_widen(old_ty: &FieldType, new_ty: &FieldType) -> bool {
    matches!(
        (old_ty, new_ty),
        (FieldType::U8,  FieldType::U16)  |
        (FieldType::U8,  FieldType::U32)  |
        (FieldType::U8,  FieldType::U64)  |
        (FieldType::U8,  FieldType::U128) |
        (FieldType::U16, FieldType::U32)  |
        (FieldType::U16, FieldType::U64)  |
        (FieldType::U16, FieldType::U128) |
        (FieldType::U32, FieldType::U64)  |
        (FieldType::U32, FieldType::U128) |
        (FieldType::U64, FieldType::U128) |
        (FieldType::I8,  FieldType::I16)  |
        (FieldType::I8,  FieldType::I32)  |
        (FieldType::I8,  FieldType::I64)  |
        (FieldType::I8,  FieldType::I128) |
        (FieldType::I16, FieldType::I32)  |
        (FieldType::I16, FieldType::I64)  |
        (FieldType::I16, FieldType::I128) |
        (FieldType::I32, FieldType::I64)  |
        (FieldType::I32, FieldType::I128) |
        (FieldType::I64, FieldType::I128)
    )
}

fn is_narrowing(old_ty: &FieldType, new_ty: &FieldType) -> bool {
    matches!(
        (old_ty, new_ty),
        (FieldType::U128, FieldType::U64)  |
        (FieldType::U128, FieldType::U32)  |
        (FieldType::U128, FieldType::U16)  |
        (FieldType::U128, FieldType::U8)   |
        (FieldType::U64,  FieldType::U32)  |
        (FieldType::U64,  FieldType::U16)  |
        (FieldType::U64,  FieldType::U8)   |
        (FieldType::U32,  FieldType::U16)  |
        (FieldType::U32,  FieldType::U8)   |
        (FieldType::U16,  FieldType::U8)   |
        (FieldType::I128, FieldType::I64)  |
        (FieldType::I128, FieldType::I32)  |
        (FieldType::I128, FieldType::I16)  |
        (FieldType::I128, FieldType::I8)   |
        (FieldType::I64,  FieldType::I32)  |
        (FieldType::I64,  FieldType::I16)  |
        (FieldType::I64,  FieldType::I8)   |
        (FieldType::I32,  FieldType::I16)  |
        (FieldType::I32,  FieldType::I8)   |
        (FieldType::I16,  FieldType::I8)   |
        (FieldType::F64,  FieldType::F32)
    )
}

fn is_sign_flip(old_ty: &FieldType, new_ty: &FieldType) -> bool {
    matches!(
        (old_ty, new_ty),
        (FieldType::U8,   FieldType::I8)   |
        (FieldType::I8,   FieldType::U8)   |
        (FieldType::U16,  FieldType::I16)  |
        (FieldType::I16,  FieldType::U16)  |
        (FieldType::U32,  FieldType::I32)  |
        (FieldType::I32,  FieldType::U32)  |
        (FieldType::U64,  FieldType::I64)  |
        (FieldType::I64,  FieldType::U64)  |
        (FieldType::U128, FieldType::I128) |
        (FieldType::I128, FieldType::U128)
    )
}

fn is_float_int_change(old_ty: &FieldType, new_ty: &FieldType) -> bool {
    let old_float = matches!(old_ty, FieldType::F32 | FieldType::F64);
    let new_float = matches!(new_ty, FieldType::F32 | FieldType::F64);
    let old_int = matches!(
        old_ty,
        FieldType::U8  | FieldType::U16 | FieldType::U32 | FieldType::U64  | FieldType::U128 |
        FieldType::I8  | FieldType::I16 | FieldType::I32 | FieldType::I64  | FieldType::I128
    );
    let new_int = matches!(
        new_ty,
        FieldType::U8  | FieldType::U16 | FieldType::U32 | FieldType::U64  | FieldType::U128 |
        FieldType::I8  | FieldType::I16 | FieldType::I32 | FieldType::I64  | FieldType::I128
    );
    (old_float && new_int) || (old_int && new_float)
}
