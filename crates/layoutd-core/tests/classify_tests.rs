/// Unit tests for the classifier — every Safety verdict for every ChangeKind path.
/// Each test builds a minimal FieldChange by hand and calls classify_one directly
/// so we can control max_new_index without relying on the diff engine.
use layoutd_core::borsh::{size_of, FieldLayout, Offset};
use layoutd_core::classify::{classify_one, Safety};
use layoutd_core::diff::{ChangeKind, FieldChange};
use layoutd_core::idl::FieldType;

// ─── helpers ──────────────────────────────────────────────────────────────────

fn fl(name: &str, ty: FieldType, index: usize) -> FieldLayout {
    let size = size_of(&ty);
    FieldLayout { name: name.to_string(), ty, index, offset: Offset::Fixed(8), size }
}

fn change(name: &str, kind: ChangeKind, old: Option<FieldLayout>, new: Option<FieldLayout>) -> FieldChange {
    FieldChange { name: name.to_string(), kind, old_layout: old, new_layout: new }
}

// ─── Unchanged ────────────────────────────────────────────────────────────────

#[test]
fn unchanged_is_safe() {
    let layout = fl("owner", FieldType::Pubkey, 0);
    let c = change("owner", ChangeKind::Unchanged, Some(layout.clone()), Some(layout));
    assert_eq!(classify_one(c, 0).safety, Safety::Safe);
}

// ─── Added ────────────────────────────────────────────────────────────────────

#[test]
fn added_at_end_is_safe() {
    // at_index == max_new_index → end-add
    let layout = fl("version", FieldType::U8, 3);
    let c = change("version", ChangeKind::Added { at_index: 3 }, None, Some(layout));
    assert_eq!(classify_one(c, 3).safety, Safety::Safe);
}

#[test]
fn added_in_middle_is_review() {
    // at_index < max_new_index → mid-add
    let layout = fl("fee_rate", FieldType::U16, 1);
    let c = change("fee_rate", ChangeKind::Added { at_index: 1 }, None, Some(layout));
    assert_eq!(classify_one(c, 3).safety, Safety::Review);
}

#[test]
fn added_as_only_field_is_safe() {
    // Single new field: at_index == 0 == max_new_index
    let layout = fl("bump", FieldType::U8, 0);
    let c = change("bump", ChangeKind::Added { at_index: 0 }, None, Some(layout));
    assert_eq!(classify_one(c, 0).safety, Safety::Safe);
}

// ─── Removed ──────────────────────────────────────────────────────────────────

#[test]
fn removed_is_danger() {
    let layout = fl("bump", FieldType::U8, 2);
    let c = change("bump", ChangeKind::Removed { from_index: 2 }, Some(layout), None);
    assert_eq!(classify_one(c, 1).safety, Safety::Danger);
}

#[test]
fn removed_middle_field_is_danger() {
    let layout = fl("fee_bps", FieldType::U16, 1);
    let c = change("fee_bps", ChangeKind::Removed { from_index: 1 }, Some(layout), None);
    assert_eq!(classify_one(c, 3).safety, Safety::Danger);
}

// ─── Reordered ────────────────────────────────────────────────────────────────

#[test]
fn reordered_is_safe_for_borsh() {
    let old = fl("bump", FieldType::U8, 1);
    let new = fl("bump", FieldType::U8, 2);
    let c = change("bump", ChangeKind::Reordered { old_index: 1, new_index: 2 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 2).safety, Safety::Safe);
}

// ─── Renamed ──────────────────────────────────────────────────────────────────

#[test]
fn renamed_is_safe() {
    let old = fl("amount", FieldType::U64, 1);
    let new = fl("balance", FieldType::U64, 1);
    let c = change(
        "balance",
        ChangeKind::Renamed { from_name: "amount".to_string() },
        Some(old),
        Some(new),
    );
    assert_eq!(classify_one(c, 2).safety, Safety::Safe);
}

// ─── TypeChanged — widening (Review) ─────────────────────────────────────────

#[test]
fn widen_u8_to_u16_is_review() {
    let old = fl("val", FieldType::U8, 0);
    let new = fl("val", FieldType::U16, 0);
    let c = change("val", ChangeKind::TypeChanged { old_type: FieldType::U8, new_type: FieldType::U16 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Review);
}

#[test]
fn widen_u32_to_u64_is_review() {
    let old = fl("balance", FieldType::U32, 0);
    let new = fl("balance", FieldType::U64, 0);
    let c = change("balance", ChangeKind::TypeChanged { old_type: FieldType::U32, new_type: FieldType::U64 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Review);
}

#[test]
fn widen_u64_to_u128_is_review() {
    let old = fl("supply", FieldType::U64, 0);
    let new = fl("supply", FieldType::U128, 0);
    let c = change("supply", ChangeKind::TypeChanged { old_type: FieldType::U64, new_type: FieldType::U128 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Review);
}

#[test]
fn widen_i32_to_i64_is_review() {
    let old = fl("delta", FieldType::I32, 0);
    let new = fl("delta", FieldType::I64, 0);
    let c = change("delta", ChangeKind::TypeChanged { old_type: FieldType::I32, new_type: FieldType::I64 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Review);
}

#[test]
fn widen_f32_to_f64_is_review() {
    let old = fl("price", FieldType::F32, 0);
    let new = fl("price", FieldType::F64, 0);
    let c = change("price", ChangeKind::TypeChanged { old_type: FieldType::F32, new_type: FieldType::F64 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Review);
}

// ─── TypeChanged — narrowing (Danger) ────────────────────────────────────────

#[test]
fn narrow_u64_to_u32_is_danger() {
    let old = fl("balance", FieldType::U64, 0);
    let new = fl("balance", FieldType::U32, 0);
    let c = change("balance", ChangeKind::TypeChanged { old_type: FieldType::U64, new_type: FieldType::U32 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn narrow_u128_to_u8_is_danger() {
    let old = fl("val", FieldType::U128, 0);
    let new = fl("val", FieldType::U8, 0);
    let c = change("val", ChangeKind::TypeChanged { old_type: FieldType::U128, new_type: FieldType::U8 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn narrow_f64_to_f32_is_danger() {
    let old = fl("ratio", FieldType::F64, 0);
    let new = fl("ratio", FieldType::F32, 0);
    let c = change("ratio", ChangeKind::TypeChanged { old_type: FieldType::F64, new_type: FieldType::F32 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

// ─── TypeChanged — sign flip (Danger) ────────────────────────────────────────

#[test]
fn sign_flip_u32_to_i32_is_danger() {
    let old = fl("count", FieldType::U32, 0);
    let new = fl("count", FieldType::I32, 0);
    let c = change("count", ChangeKind::TypeChanged { old_type: FieldType::U32, new_type: FieldType::I32 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn sign_flip_i64_to_u64_is_danger() {
    let old = fl("ts", FieldType::I64, 0);
    let new = fl("ts", FieldType::U64, 0);
    let c = change("ts", ChangeKind::TypeChanged { old_type: FieldType::I64, new_type: FieldType::U64 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn sign_flip_u8_to_i8_is_danger() {
    let old = fl("b", FieldType::U8, 0);
    let new = fl("b", FieldType::I8, 0);
    let c = change("b", ChangeKind::TypeChanged { old_type: FieldType::U8, new_type: FieldType::I8 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

// ─── TypeChanged — float ↔ integer (Danger) ───────────────────────────────────

#[test]
fn float_to_int_is_danger() {
    let old = fl("rate", FieldType::F64, 0);
    let new = fl("rate", FieldType::U64, 0);
    let c = change("rate", ChangeKind::TypeChanged { old_type: FieldType::F64, new_type: FieldType::U64 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn int_to_float_is_danger() {
    let old = fl("price", FieldType::U32, 0);
    let new = fl("price", FieldType::F32, 0);
    let c = change("price", ChangeKind::TypeChanged { old_type: FieldType::U32, new_type: FieldType::F32 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

// ─── TypeChanged — string reinterpretation (Danger) ──────────────────────────

#[test]
fn string_to_pubkey_is_danger() {
    let old = fl("label", FieldType::String, 0);
    let new = fl("label", FieldType::Pubkey, 0);
    let c = change("label", ChangeKind::TypeChanged { old_type: FieldType::String, new_type: FieldType::Pubkey }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn pubkey_to_string_is_danger() {
    let old = fl("addr", FieldType::Pubkey, 0);
    let new = fl("addr", FieldType::String, 0);
    let c = change("addr", ChangeKind::TypeChanged { old_type: FieldType::Pubkey, new_type: FieldType::String }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

// ─── TypeChanged — vec reinterpretation (Danger) ─────────────────────────────

#[test]
fn vec_to_non_vec_is_danger() {
    let old = fl("items", FieldType::Vec(Box::new(FieldType::U64)), 0);
    let new = fl("items", FieldType::U64, 0);
    let c = change(
        "items",
        ChangeKind::TypeChanged {
            old_type: FieldType::Vec(Box::new(FieldType::U64)),
            new_type: FieldType::U64,
        },
        Some(old),
        Some(new),
    );
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn non_vec_to_vec_is_danger() {
    let old = fl("count", FieldType::U64, 0);
    let new = fl("count", FieldType::Vec(Box::new(FieldType::U64)), 0);
    let c = change(
        "count",
        ChangeKind::TypeChanged {
            old_type: FieldType::U64,
            new_type: FieldType::Vec(Box::new(FieldType::U64)),
        },
        Some(old),
        Some(new),
    );
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

// ─── TypeChanged — unknown type (Danger) ─────────────────────────────────────

#[test]
fn unknown_old_type_is_danger() {
    let old = fl("data", FieldType::Unknown("SomeWeirdType".to_string()), 0);
    let new = fl("data", FieldType::U64, 0);
    let c = change(
        "data",
        ChangeKind::TypeChanged {
            old_type: FieldType::Unknown("SomeWeirdType".to_string()),
            new_type: FieldType::U64,
        },
        Some(old),
        Some(new),
    );
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

#[test]
fn unknown_new_type_is_danger() {
    let old = fl("data", FieldType::U64, 0);
    let new = fl("data", FieldType::Unknown("NewWeirdType".to_string()), 0);
    let c = change(
        "data",
        ChangeKind::TypeChanged {
            old_type: FieldType::U64,
            new_type: FieldType::Unknown("NewWeirdType".to_string()),
        },
        Some(old),
        Some(new),
    );
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

// ─── TypeChanged — generic reinterpretation catch-all (Danger) ───────────────

#[test]
fn pubkey_to_u64_is_danger() {
    let old = fl("key", FieldType::Pubkey, 0);
    let new = fl("key", FieldType::U64, 0);
    let c = change("key", ChangeKind::TypeChanged { old_type: FieldType::Pubkey, new_type: FieldType::U64 }, Some(old), Some(new));
    assert_eq!(classify_one(c, 0).safety, Safety::Danger);
}

// ─── TypeChangedAndReordered ──────────────────────────────────────────────────

#[test]
fn type_changed_and_reordered_widen_is_review() {
    let old = fl("count", FieldType::U32, 2);
    let new = fl("count", FieldType::U64, 0);
    let c = change(
        "count",
        ChangeKind::TypeChangedAndReordered {
            old_type: FieldType::U32,
            old_index: 2,
            new_type: FieldType::U64,
            new_index: 0,
        },
        Some(old),
        Some(new),
    );
    assert_eq!(classify_one(c, 2).safety, Safety::Review);
}

#[test]
fn type_changed_and_reordered_sign_flip_is_danger() {
    let old = fl("amount", FieldType::U64, 1);
    let new = fl("amount", FieldType::I64, 0);
    let c = change(
        "amount",
        ChangeKind::TypeChangedAndReordered {
            old_type: FieldType::U64,
            old_index: 1,
            new_type: FieldType::I64,
            new_index: 0,
        },
        Some(old),
        Some(new),
    );
    assert_eq!(classify_one(c, 2).safety, Safety::Danger);
}

// ─── classify_all — context-derived position ──────────────────────────────────

#[test]
fn classify_all_correctly_identifies_end_add() {
    use layoutd_core::classify::classify_all;

    let old_layout = fl("owner", FieldType::Pubkey, 0);
    let new_end   = fl("bump",  FieldType::U8,     1); // index 1 == max new index

    let changes = vec![
        FieldChange {
            name: "owner".to_string(),
            kind: ChangeKind::Unchanged,
            old_layout: Some(old_layout.clone()),
            new_layout: Some(fl("owner", FieldType::Pubkey, 0)),
        },
        FieldChange {
            name: "bump".to_string(),
            kind: ChangeKind::Added { at_index: 1 },
            old_layout: None,
            new_layout: Some(new_end),
        },
    ];

    let classified = classify_all(changes);
    let added = classified.iter().find(|c| c.change.name == "bump").unwrap();
    assert_eq!(added.safety, Safety::Safe);
}

#[test]
fn classify_all_correctly_identifies_mid_add() {
    use layoutd_core::classify::classify_all;

    // "fee_rate" is at index 1, but "bump" is at index 2 → fee_rate is mid-add
    let changes = vec![
        FieldChange {
            name: "fee_rate".to_string(),
            kind: ChangeKind::Added { at_index: 1 },
            old_layout: None,
            new_layout: Some(fl("fee_rate", FieldType::U16, 1)),
        },
        FieldChange {
            name: "bump".to_string(),
            kind: ChangeKind::Unchanged,
            old_layout: Some(fl("bump", FieldType::U8, 2)),
            new_layout: Some(fl("bump", FieldType::U8, 2)),
        },
    ];

    let classified = classify_all(changes);
    let added = classified.iter().find(|c| c.change.name == "fee_rate").unwrap();
    assert_eq!(added.safety, Safety::Review);
}

// ─── Negative corpus — must never produce Safe for these ─────────────────────

#[test]
fn negative_corpus_narrowing_never_safe() {
    let pairs = vec![
        (FieldType::U128, FieldType::U64),
        (FieldType::U64,  FieldType::U32),
        (FieldType::U32,  FieldType::U16),
        (FieldType::U16,  FieldType::U8),
        (FieldType::I64,  FieldType::I32),
        (FieldType::F64,  FieldType::F32),
    ];
    for (old_ty, new_ty) in pairs {
        let old = fl("x", old_ty.clone(), 0);
        let new = fl("x", new_ty.clone(), 0);
        let c = change("x", ChangeKind::TypeChanged { old_type: old_ty, new_type: new_ty }, Some(old), Some(new));
        let result = classify_one(c, 0);
        assert_ne!(result.safety, Safety::Safe, "narrowing must never be Safe: {:?}", result.reason);
    }
}

#[test]
fn negative_corpus_sign_flip_never_safe() {
    let pairs = vec![
        (FieldType::U8,  FieldType::I8),
        (FieldType::U32, FieldType::I32),
        (FieldType::U64, FieldType::I64),
        (FieldType::I8,  FieldType::U8),
        (FieldType::I64, FieldType::U64),
    ];
    for (old_ty, new_ty) in pairs {
        let old = fl("x", old_ty.clone(), 0);
        let new = fl("x", new_ty.clone(), 0);
        let c = change("x", ChangeKind::TypeChanged { old_type: old_ty, new_type: new_ty }, Some(old), Some(new));
        let result = classify_one(c, 0);
        assert_ne!(result.safety, Safety::Safe, "sign-flip must never be Safe: {:?}", result.reason);
    }
}

#[test]
fn negative_corpus_removal_never_safe_or_review() {
    let layout = fl("secret_key", FieldType::Pubkey, 0);
    let c = change("secret_key", ChangeKind::Removed { from_index: 0 }, Some(layout), None);
    let result = classify_one(c, 0);
    assert_eq!(result.safety, Safety::Danger, "removal must always be Danger");
}
