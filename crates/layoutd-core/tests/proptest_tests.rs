/// Property-based tests for the Borsh and zero-copy classifiers.
///
/// Each test encodes a *universal claim* about the classifier — not a single
/// hand-crafted scenario but a rule that must hold for *any* input that
/// satisfies the strategy constraints.  Proptest generates hundreds of random
/// inputs per run and shrinks any failure to a minimal counter-example.
use proptest::prelude::*;

use layoutd_core::borsh::compute_layout;
use layoutd_core::classify::{classify_all, Safety};
use layoutd_core::diff::{diff, ChangeKind};
use layoutd_core::idl::{AccountDef, FieldDef, FieldType};
use layoutd_core::zerocopy::{classify_zc_all, compute_zc_layout, zc_to_borsh_layout};

// ── strategies ────────────────────────────────────────────────────────────────

/// Arbitrary primitive type that is valid in both Borsh and zero-copy structs.
fn arb_fixed_type() -> impl Strategy<Value = FieldType> {
    prop_oneof![
        Just(FieldType::U8),
        Just(FieldType::U16),
        Just(FieldType::U32),
        Just(FieldType::U64),
        Just(FieldType::U128),
        Just(FieldType::I8),
        Just(FieldType::I16),
        Just(FieldType::I32),
        Just(FieldType::I64),
        Just(FieldType::I128),
        Just(FieldType::Bool),
        Just(FieldType::F32),
        Just(FieldType::F64),
        Just(FieldType::Pubkey),
    ]
}

/// Arbitrary FieldType including variable-length ones (Borsh-only).
fn arb_any_type() -> impl Strategy<Value = FieldType> {
    prop_oneof![
        4 => arb_fixed_type(),
        1 => Just(FieldType::String),
        1 => arb_fixed_type().prop_map(|t| FieldType::Vec(Box::new(t))),
    ]
}

/// AccountDef with 1–5 fields, all fixed-size types (valid for both modes).
fn arb_fixed_account() -> impl Strategy<Value = AccountDef> {
    (1usize..=5).prop_flat_map(|n| {
        proptest::collection::vec(arb_fixed_type(), n).prop_map(|types| {
            let fields = types
                .into_iter()
                .enumerate()
                .map(|(i, ty)| FieldDef { name: format!("f{i}"), ty, index: i })
                .collect();
            AccountDef { name: "TestAccount".to_string(), fields }
        })
    })
}

/// AccountDef with 1–6 fields, any types (Borsh mode only).
fn arb_any_account() -> impl Strategy<Value = AccountDef> {
    (1usize..=6).prop_flat_map(|n| {
        proptest::collection::vec(arb_any_type(), n).prop_map(|types| {
            let fields = types
                .into_iter()
                .enumerate()
                .map(|(i, ty)| FieldDef { name: format!("f{i}"), ty, index: i })
                .collect();
            AccountDef { name: "TestAccount".to_string(), fields }
        })
    })
}

// ── Borsh classifier properties ───────────────────────────────────────────────

proptest! {
    /// Comparing any account definition against itself yields only Safe results.
    #[test]
    fn prop_identical_layouts_are_all_safe(def in arb_any_account()) {
        let layout = compute_layout(&def);
        let cs = classify_all(diff(&layout, &layout));
        prop_assert!(
            cs.iter().all(|c| c.safety == Safety::Safe),
            "unexpected non-safe result in identical diff: {:?}",
            cs.iter().filter(|c| c.safety != Safety::Safe).collect::<Vec<_>>()
        );
    }

    /// Adding one field at the end is always Safe in Borsh.
    #[test]
    fn prop_add_at_end_is_always_safe(
        base in arb_any_account(),
        new_ty in arb_any_type(),
    ) {
        let n = base.fields.len();
        let mut new_def = base.clone();
        new_def.fields.push(FieldDef { name: "appended".to_string(), ty: new_ty, index: n });

        let cs = classify_all(diff(&compute_layout(&base), &compute_layout(&new_def)));
        let added = cs.iter().find(|c| c.change.name == "appended").expect("added field not in diff");
        prop_assert_eq!(
            added.safety, Safety::Safe,
            "add-at-end should be Safe; reason: {}", added.reason
        );
    }

    /// Renaming a field (same type, same position) is always Safe in Borsh.
    #[test]
    fn prop_rename_is_always_safe(
        base in arb_any_account().prop_filter("need ≥1 field", |d| !d.fields.is_empty()),
        idx in 0usize..6,
    ) {
        let idx = idx % base.fields.len();
        let mut new_def = base.clone();
        new_def.fields[idx].name = format!("renamed_{idx}");

        let cs = classify_all(diff(&compute_layout(&base), &compute_layout(&new_def)));
        let target_name = format!("renamed_{idx}");
        let found = cs.iter().find(|c| c.change.name == target_name)
            .expect("renamed field not in diff");
        prop_assert_eq!(
            found.safety, Safety::Safe,
            "rename should be Safe; got {:?}: {}", found.safety, found.reason
        );
    }

    /// Removing any field always produces at least one Danger result.
    #[test]
    fn prop_remove_always_produces_danger(
        base in arb_any_account().prop_filter("need ≥1 field", |d| !d.fields.is_empty()),
        rm_idx in 0usize..6,
    ) {
        let rm_idx = rm_idx % base.fields.len();
        let removed_name = base.fields[rm_idx].name.clone();

        let mut new_def = base.clone();
        new_def.fields.remove(rm_idx);
        // re-index
        for (i, f) in new_def.fields.iter_mut().enumerate() { f.index = i; }

        let cs = classify_all(diff(&compute_layout(&base), &compute_layout(&new_def)));
        let removed_change = cs.iter().find(|c| c.change.name == removed_name)
            .expect("removed field not in diff");
        prop_assert_eq!(
            removed_change.safety, Safety::Danger,
            "remove should always be Danger"
        );
    }

    /// Reordering fields is always Safe in Borsh (serialization matches by name).
    #[test]
    fn prop_borsh_reorder_is_always_safe(
        base in arb_any_account().prop_filter("need ≥2 fields", |d| d.fields.len() >= 2),
    ) {
        // Reverse field order → maximum reordering.
        let mut new_def = base.clone();
        new_def.fields.reverse();
        for (i, f) in new_def.fields.iter_mut().enumerate() { f.index = i; }

        let cs = classify_all(diff(&compute_layout(&base), &compute_layout(&new_def)));
        let non_safe_reorders: Vec<_> = cs.iter()
            .filter(|c| matches!(c.change.kind, ChangeKind::Reordered { .. }) && c.safety != Safety::Safe)
            .collect();
        prop_assert!(
            non_safe_reorders.is_empty(),
            "Borsh reorder should always be Safe; non-safe: {:?}", non_safe_reorders
        );
    }

    /// Adding a field in the middle is at most Review (never Danger) in Borsh.
    #[test]
    fn prop_borsh_mid_insert_at_most_review(
        base in arb_any_account().prop_filter("need ≥2 fields for 'middle'", |d| d.fields.len() >= 2),
        new_ty in arb_any_type(),
    ) {
        // Insert at position 1 (definitively "middle" when there's a last field after it)
        let mut new_fields = base.fields.clone();
        new_fields.insert(1, FieldDef { name: "mid".to_string(), ty: new_ty, index: 1 });
        for (i, f) in new_fields.iter_mut().enumerate() { f.index = i; }
        let new_def = AccountDef { name: base.name.clone(), fields: new_fields };

        let cs = classify_all(diff(&compute_layout(&base), &compute_layout(&new_def)));
        let mid = cs.iter().find(|c| c.change.name == "mid").expect("mid not in diff");
        prop_assert!(
            mid.safety != Safety::Danger,
            "Borsh mid-insert should be at most Review, got Danger: {}", mid.reason
        );
    }
}

// ── Zero-copy classifier properties ──────────────────────────────────────────

proptest! {
    /// Comparing any fixed-type account against itself gives all Safe results in ZC mode.
    #[test]
    fn prop_zc_identical_is_all_safe(def in arb_fixed_account()) {
        let zc = compute_zc_layout(&def).unwrap();
        let bl = zc_to_borsh_layout(&zc);
        let cs = classify_zc_all(diff(&bl, &bl), &zc, &zc);
        prop_assert!(cs.iter().all(|c| c.safety == Safety::Safe));
    }

    /// Adding a fixed-type field at the end is always Safe in ZC.
    #[test]
    fn prop_zc_add_at_end_is_always_safe(
        base in arb_fixed_account(),
        new_ty in arb_fixed_type(),
    ) {
        let n = base.fields.len();
        let mut new_def = base.clone();
        new_def.fields.push(FieldDef { name: "appended".to_string(), ty: new_ty, index: n });

        let old_zc = compute_zc_layout(&base).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let cs = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let added = cs.iter().find(|c| c.change.name == "appended").unwrap();
        prop_assert_eq!(added.safety, Safety::Safe);
    }

    /// Renaming a field (same type, same position) is always Safe in ZC.
    #[test]
    fn prop_zc_rename_is_always_safe(
        base in arb_fixed_account().prop_filter("need ≥1 field", |d| !d.fields.is_empty()),
        idx in 0usize..5,
    ) {
        let idx = idx % base.fields.len();
        let mut new_def = base.clone();
        new_def.fields[idx].name = format!("renamed_{idx}");

        let old_zc = compute_zc_layout(&base).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let cs = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let target = format!("renamed_{idx}");
        let found = cs.iter().find(|c| c.change.name == target).unwrap();
        prop_assert_eq!(found.safety, Safety::Safe);
    }

    /// Removing any field always produces Danger in ZC.
    #[test]
    fn prop_zc_remove_always_danger(
        base in arb_fixed_account().prop_filter("need ≥1 field", |d| !d.fields.is_empty()),
        rm_idx in 0usize..5,
    ) {
        let rm_idx = rm_idx % base.fields.len();
        let removed_name = base.fields[rm_idx].name.clone();

        let mut new_def = base.clone();
        new_def.fields.remove(rm_idx);
        for (i, f) in new_def.fields.iter_mut().enumerate() { f.index = i; }

        let old_zc = compute_zc_layout(&base).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let cs = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let removed = cs.iter().find(|c| c.change.name == removed_name).unwrap();
        prop_assert_eq!(removed.safety, Safety::Danger);
    }

    /// Reordering any fields is always Danger in ZC (offsets are position-dependent).
    #[test]
    fn prop_zc_reorder_is_always_danger(
        base in arb_fixed_account().prop_filter("need ≥2 fields", |d| d.fields.len() >= 2),
    ) {
        let mut new_def = base.clone();
        new_def.fields.reverse();
        for (i, f) in new_def.fields.iter_mut().enumerate() { f.index = i; }

        let old_zc = compute_zc_layout(&base).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let cs = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let reordered: Vec<_> = cs.iter()
            .filter(|c| matches!(c.change.kind, ChangeKind::Reordered { .. }))
            .collect();
        // There must be at least one reordered field, and all must be Danger.
        prop_assert!(!reordered.is_empty(), "expected reordered changes in reversed struct");
        prop_assert!(
            reordered.iter().all(|c| c.safety == Safety::Danger),
            "all ZC reordered fields must be Danger"
        );
    }

    /// Inserting a field in the middle is always Danger in ZC.
    #[test]
    fn prop_zc_mid_insert_is_always_danger(
        base in arb_fixed_account().prop_filter("need ≥2 fields", |d| d.fields.len() >= 2),
        new_ty in arb_fixed_type(),
    ) {
        let mut new_fields = base.fields.clone();
        new_fields.insert(1, FieldDef { name: "mid".to_string(), ty: new_ty, index: 1 });
        for (i, f) in new_fields.iter_mut().enumerate() { f.index = i; }
        let new_def = AccountDef { name: base.name.clone(), fields: new_fields };

        let old_zc = compute_zc_layout(&base).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let cs = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let mid = cs.iter().find(|c| c.change.name == "mid").unwrap();
        prop_assert_eq!(mid.safety, Safety::Danger);
    }

    /// ZC field offsets are always strictly increasing.
    #[test]
    fn prop_zc_offsets_are_strictly_increasing(def in arb_fixed_account()) {
        let zc = compute_zc_layout(&def).unwrap();
        for w in zc.fields.windows(2) {
            prop_assert!(
                w[1].offset > w[0].offset,
                "offsets must be strictly increasing: {} offset {} vs {} offset {}",
                w[0].name, w[0].offset, w[1].name, w[1].offset,
            );
        }
    }

    /// ZC each field's offset satisfies its alignment requirement.
    #[test]
    fn prop_zc_offsets_satisfy_alignment(def in arb_fixed_account()) {
        let zc = compute_zc_layout(&def).unwrap();
        for f in &zc.fields {
            // The struct-internal offset is (f.offset - 8).
            // For alignment: (f.offset - 8) % f.align == 0
            let struct_off = f.offset - 8;
            prop_assert_eq!(
                struct_off % f.align, 0,
                "field '{}' at offset {} (struct-internal {}) violates alignment {}",
                f.name, f.offset, struct_off, f.align
            );
        }
    }

    /// ZC total_size is always a multiple of struct_align.
    #[test]
    fn prop_zc_total_size_is_multiple_of_struct_align(def in arb_fixed_account()) {
        let zc = compute_zc_layout(&def).unwrap();
        let struct_body = zc.total_size - 8;
        prop_assert_eq!(
            struct_body % zc.struct_align, 0,
            "struct body {} is not a multiple of struct_align {}",
            struct_body, zc.struct_align
        );
    }

    /// Borsh and ZC agree on Safe verdict for add-at-end.
    /// (Both classify adding at end as Safe — they only disagree on reorder / mid-insert.)
    #[test]
    fn prop_borsh_and_zc_agree_on_add_at_end(
        base in arb_fixed_account(),
        new_ty in arb_fixed_type(),
    ) {
        let n = base.fields.len();
        let mut new_def = base.clone();
        new_def.fields.push(FieldDef { name: "appended".to_string(), ty: new_ty, index: n });

        // Borsh
        let borsh_cs = classify_all(diff(&compute_layout(&base), &compute_layout(&new_def)));
        let borsh_added = borsh_cs.iter().find(|c| c.change.name == "appended").unwrap();

        // ZC
        let old_zc = compute_zc_layout(&base).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let zc_cs = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let zc_added = zc_cs.iter().find(|c| c.change.name == "appended").unwrap();

        prop_assert_eq!(borsh_added.safety, Safety::Safe);
        prop_assert_eq!(zc_added.safety, Safety::Safe);
    }
}
