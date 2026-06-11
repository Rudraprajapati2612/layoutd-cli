use crate::diff::{ChangeKind, FieldChange};
use crate::idl::FieldType;

#[derive(Debug, Clone, PartialEq)]
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
/// Determines position context (is a field truly at the end?) from the list itself.
pub fn classify_all(changes: Vec<FieldChange>) -> Vec<ClassifiedChange> {
    let max_new_index = changes
        .iter()
        .filter_map(|c| c.new_layout.as_ref())
        .map(|fl| fl.index)
        .max()
        .unwrap_or(0);

    changes.into_iter().map(|c| classify_one(c, max_new_index)).collect()
}

/// Classify a single change. `max_new_index` is the highest field index in the new layout,
/// used to decide whether an Added field is at the end (Safe) or in the middle (Review).
pub fn classify_one(change: FieldChange, max_new_index: usize) -> ClassifiedChange {
    let (safety, reason) = match &change.kind {
        ChangeKind::Unchanged => (Safety::Safe, "field unchanged"),

        ChangeKind::Added { at_index } => {
            if *at_index >= max_new_index {
                (Safety::Safe, "field added at end — no existing offsets shift")
            } else {
                (
                    Safety::Review,
                    "field added in middle — safe for Borsh but verify alignment for zero-copy",
                )
            }
        }

        ChangeKind::Removed { .. } => (
            Safety::Danger,
            "field removed — permanent data loss; suggest marking deprecated instead",
        ),

        ChangeKind::Reordered { .. } => (
            Safety::Safe,
            "field reordered — safe for Borsh, serialization matches by name",
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
    // Float widen is value-preserving but gets its own message.
    if matches!((old_ty, new_ty), (FieldType::F32, FieldType::F64)) {
        return (
            Safety::Review,
            "float widen f32→f64 — value preserved, verify all producers and consumers updated",
        );
    }

    if is_safe_widen(old_ty, new_ty) {
        return (
            Safety::Review,
            "integer widen — value fits in larger type, verify no signedness assumption",
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
