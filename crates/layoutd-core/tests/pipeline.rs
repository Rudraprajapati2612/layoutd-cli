/// End-to-end pipeline tests: IDL → parse → Borsh layout → diff → classify.
/// Real accounts (EscrowVault, Market) from idl.json serve as the base.
/// Modified versions are built in-memory so we don't need extra JSON files.
use std::collections::HashMap;
use std::path::Path;

use layoutd_core::borsh::{compute_layout, FieldLayout, Offset, Size};
use layoutd_core::classify::{classify_all, Safety};
use layoutd_core::diff::{diff, ChangeKind};
use layoutd_core::idl::{parse_idl, AccountDef, FieldDef, FieldType};

// ─── helpers ──────────────────────────────────────────────────────────────────

fn idl_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../idl.json")
}

fn escrow_vault() -> AccountDef {
    parse_idl(&idl_path(), "EscrowVault").unwrap()
}

fn market() -> AccountDef {
    parse_idl(&idl_path(), "Market").unwrap()
}

/// Build an AccountDef from a plain field list; indices are assigned by position.
fn make_account(name: &str, fields: Vec<(&str, FieldType)>) -> AccountDef {
    AccountDef {
        name: name.to_string(),
        fields: fields
            .into_iter()
            .enumerate()
            .map(|(i, (n, ty))| FieldDef { name: n.to_string(), ty, index: i })
            .collect(),
    }
}

/// Re-index fields after adding/removing entries so indices stay contiguous.
fn reindex(fields: Vec<FieldDef>) -> Vec<FieldDef> {
    fields.into_iter().enumerate().map(|(i, mut f)| { f.index = i; f }).collect()
}

fn by_name(fields: &[FieldLayout]) -> HashMap<&str, &FieldLayout> {
    fields.iter().map(|f| (f.name.as_str(), f)).collect()
}

// ─── Layout: EscrowVault ──────────────────────────────────────────────────────

#[test]
fn escrow_vault_total_size_is_227() {
    // 8 discriminator
    // 5 × Pubkey (32) = 160   (market, mrarket_registery_program, usdc_vault, yes_token_mint, no_token_mint)
    // 3 × U64   ( 8) =  24   (total_locked_collateral, total_yes_minted, total_no_minted)
    // 2 × Bool  ( 1) =   2   (is_settled, is_minting_paused)
    // 1 × Pubkey(32) =  32   (admin)
    // 1 × U8    ( 1) =   1   (bump)
    // total = 8 + 160 + 24 + 2 + 32 + 1 = 227
    let layout = compute_layout(&escrow_vault());
    assert_eq!(layout.total_size, Size::Fixed(227));
}

#[test]
fn escrow_vault_field_offsets() {
    let layout = compute_layout(&escrow_vault());
    let m = by_name(&layout.fields);

    assert_eq!(m["market"].offset,                    Offset::Fixed(8));
    assert_eq!(m["mrarket_registery_program"].offset, Offset::Fixed(40));
    assert_eq!(m["usdc_vault"].offset,                Offset::Fixed(72));
    assert_eq!(m["yes_token_mint"].offset,            Offset::Fixed(104));
    assert_eq!(m["no_token_mint"].offset,             Offset::Fixed(136));
    assert_eq!(m["total_locked_collateral"].offset,   Offset::Fixed(168));
    assert_eq!(m["total_yes_minted"].offset,          Offset::Fixed(176));
    assert_eq!(m["total_no_minted"].offset,           Offset::Fixed(184));
    assert_eq!(m["is_settled"].offset,                Offset::Fixed(192));
    assert_eq!(m["is_minting_paused"].offset,         Offset::Fixed(193));
    assert_eq!(m["admin"].offset,                     Offset::Fixed(194));
    assert_eq!(m["bump"].offset,                      Offset::Fixed(226));
}

// ─── Layout: Market ───────────────────────────────────────────────────────────

#[test]
fn market_total_size_is_variable() {
    // Market has String fields → variable total
    let layout = compute_layout(&market());
    assert_eq!(layout.total_size, Size::Variable);
}

#[test]
fn market_first_two_fields_have_fixed_offsets() {
    let layout = compute_layout(&market());
    // market_id: [u8; 32] — Fixed(32), offset Fixed(8)
    // question: String — Variable, offset Fixed(40) (still reachable, but hit_variable after)
    assert_eq!(layout.fields[0].offset, Offset::Fixed(8));  // market_id
    assert_eq!(layout.fields[1].offset, Offset::Fixed(40)); // question
}

#[test]
fn market_fields_after_first_string_are_after_variable() {
    let layout = compute_layout(&market());
    // description is the 3rd field and follows question (String) → AfterVariable
    assert_eq!(layout.fields[2].offset, Offset::AfterVariable);
    // all subsequent fields are also AfterVariable
    for f in &layout.fields[2..] {
        assert_eq!(f.offset, Offset::AfterVariable, "field '{}' should be AfterVariable", f.name);
    }
}

// ─── Pipeline: no changes ─────────────────────────────────────────────────────

#[test]
fn escrow_vault_diffed_with_itself_is_all_unchanged() {
    let layout = compute_layout(&escrow_vault());
    let classified = classify_all(diff(&layout, &layout));
    assert!(classified.iter().all(|c| c.safety == Safety::Safe));
    assert!(classified.iter().all(|c| matches!(c.change.kind, ChangeKind::Unchanged)));
}

#[test]
fn market_diffed_with_itself_is_all_unchanged() {
    let layout = compute_layout(&market());
    let classified = classify_all(diff(&layout, &layout));
    assert!(classified.iter().all(|c| c.safety == Safety::Safe));
    assert!(classified.iter().all(|c| matches!(c.change.kind, ChangeKind::Unchanged)));
}

// ─── Pipeline: field added at end of exact-sized account → Danger ───────────

#[test]
fn add_field_at_end_of_escrow_vault_is_danger() {
    // EscrowVault is fixed-size (227 bytes). Old accounts have no bytes for "version".
    let old_def = escrow_vault();
    let mut new_fields = old_def.fields.clone();
    let idx = new_fields.len();
    new_fields.push(FieldDef { name: "version".to_string(), ty: FieldType::U8, index: idx });
    let new_def = AccountDef { name: old_def.name.clone(), fields: new_fields };

    let classified = classify_all(diff(
        &compute_layout(&old_def),
        &compute_layout(&new_def),
    ));

    let added = classified.iter().find(|c| c.change.name == "version").unwrap();
    assert_eq!(added.safety, Safety::Danger);
    assert!(matches!(added.change.kind, ChangeKind::Added { .. }));
}

#[test]
fn add_two_fields_at_end_both_danger_for_exact_sized_account() {
    // Old is fixed-size: authority(32) + balance(8) = exact allocation.
    // Neither fee_bps nor bump have bytes in old accounts → both DANGER.
    let old = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("balance",   FieldType::U64),
    ]);
    let new = make_account("Vault", vec![
        ("authority",  FieldType::Pubkey),
        ("balance",    FieldType::U64),
        ("fee_bps",    FieldType::U16),
        ("bump",       FieldType::U8),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let added: Vec<_> = classified.iter()
        .filter(|c| matches!(c.change.kind, ChangeKind::Added { .. }))
        .collect();
    assert!(added.iter().any(|c| c.change.name == "bump" && c.safety == Safety::Danger));
    assert!(added.iter().any(|c| c.change.name == "fee_bps" && c.safety == Safety::Danger));
}

// ─── Pipeline: field added in middle → Review ────────────────────────────────

#[test]
fn add_field_in_middle_is_review() {
    let old = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("balance",   FieldType::U64),
        ("bump",      FieldType::U8),
    ]);
    let new = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("fee_rate",  FieldType::U16),   // inserted before balance
        ("balance",   FieldType::U64),
        ("bump",      FieldType::U8),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let fee = classified.iter().find(|c| c.change.name == "fee_rate").unwrap();
    assert_eq!(fee.safety, Safety::Review);
}

// ─── Pipeline: field removed → Danger ────────────────────────────────────────

#[test]
fn remove_last_field_of_escrow_vault_is_danger() {
    let old_def = escrow_vault();
    let new_def = AccountDef {
        name: old_def.name.clone(),
        fields: reindex(
            old_def.fields.iter().filter(|f| f.name != "bump").cloned().collect()
        ),
    };

    let classified = classify_all(diff(
        &compute_layout(&old_def),
        &compute_layout(&new_def),
    ));

    let removed = classified.iter().find(|c| c.change.name == "bump").unwrap();
    assert_eq!(removed.safety, Safety::Danger);
}

#[test]
fn remove_middle_field_is_danger() {
    let old = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("balance",   FieldType::U64),
        ("bump",      FieldType::U8),
    ]);
    let new = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        // balance removed
        ("bump",      FieldType::U8),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let removed = classified.iter().find(|c| c.change.name == "balance").unwrap();
    assert_eq!(removed.safety, Safety::Danger);
}

// ─── Pipeline: widen type → Review ───────────────────────────────────────────

#[test]
fn widen_u64_to_u128_in_escrow_vault_is_review() {
    let old_def = escrow_vault();
    let new_def = AccountDef {
        name: old_def.name.clone(),
        fields: old_def.fields.iter().map(|f| {
            if f.name == "total_locked_collateral" {
                FieldDef { ty: FieldType::U128, ..f.clone() }
            } else {
                f.clone()
            }
        }).collect(),
    };

    let classified = classify_all(diff(
        &compute_layout(&old_def),
        &compute_layout(&new_def),
    ));

    let changed = classified.iter().find(|c| c.change.name == "total_locked_collateral").unwrap();
    assert_eq!(changed.safety, Safety::Danger);
    assert!(matches!(changed.change.kind, ChangeKind::TypeChanged { .. }));
}

#[test]
fn widen_u32_to_u64_is_danger() {
    let old = make_account("Vault", vec![
        ("owner",   FieldType::Pubkey),
        ("balance", FieldType::U32),
        ("bump",    FieldType::U8),
    ]);
    let new = make_account("Vault", vec![
        ("owner",   FieldType::Pubkey),
        ("balance", FieldType::U64),
        ("bump",    FieldType::U8),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let changed = classified.iter().find(|c| c.change.name == "balance").unwrap();
    assert_eq!(changed.safety, Safety::Danger);
}

// ─── Pipeline: sign flip → Danger ────────────────────────────────────────────

#[test]
fn sign_flip_u64_to_i64_is_danger() {
    let old = make_account("Vault", vec![
        ("owner",     FieldType::Pubkey),
        ("timestamp", FieldType::U64),
    ]);
    let new = make_account("Vault", vec![
        ("owner",     FieldType::Pubkey),
        ("timestamp", FieldType::I64),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let changed = classified.iter().find(|c| c.change.name == "timestamp").unwrap();
    assert_eq!(changed.safety, Safety::Danger);
}

// ─── Pipeline: narrowing → Danger ────────────────────────────────────────────

#[test]
fn narrow_u64_to_u32_is_danger() {
    let old = make_account("Vault", vec![("supply", FieldType::U64)]);
    let new = make_account("Vault", vec![("supply", FieldType::U32)]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    assert_eq!(classified[0].safety, Safety::Danger);
}

// ─── Pipeline: rename → Safe ──────────────────────────────────────────────────

#[test]
fn rename_field_same_type_and_index_is_safe() {
    let old = make_account("Vault", vec![
        ("owner",  FieldType::Pubkey),
        ("amount", FieldType::U64),
        ("bump",   FieldType::U8),
    ]);
    let new = make_account("Vault", vec![
        ("owner",   FieldType::Pubkey),
        ("balance", FieldType::U64), // renamed from amount
        ("bump",    FieldType::U8),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let renamed = classified.iter().find(|c| c.change.name == "balance").unwrap();
    assert_eq!(renamed.safety, Safety::Safe);
    assert!(
        matches!(&renamed.change.kind, ChangeKind::Renamed { from_name } if from_name == "amount"),
        "expected Renamed {{ from_name: amount }}, got {:?}", renamed.change.kind
    );
}

#[test]
fn rename_with_type_change_is_not_a_rename() {
    // Different type → must produce Removed + Added, never Renamed
    let old = make_account("Vault", vec![
        ("owner",  FieldType::Pubkey),
        ("amount", FieldType::U32),
        ("bump",   FieldType::U8),
    ]);
    let new = make_account("Vault", vec![
        ("owner",   FieldType::Pubkey),
        ("balance", FieldType::U64), // type changed too → Remove + Add
        ("bump",    FieldType::U8),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    assert!(!classified.iter().any(|c| matches!(c.change.kind, ChangeKind::Renamed { .. })));
    assert!(classified.iter().any(|c| c.change.name == "amount" && matches!(c.change.kind, ChangeKind::Removed { .. })));
    assert!(classified.iter().any(|c| c.change.name == "balance" && matches!(c.change.kind, ChangeKind::Added { .. })));
}

// ─── Pipeline: reorder → Danger (Borsh is positional) ───────────────────────

#[test]
fn reorder_fields_is_danger_for_borsh() {
    let old = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("bump",      FieldType::U8),
        ("balance",   FieldType::U64),
    ]);
    let new = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("balance",   FieldType::U64), // moved up — Borsh byte position changes
        ("bump",      FieldType::U8),
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let reordered: Vec<_> = classified.iter()
        .filter(|c| matches!(c.change.kind, ChangeKind::Reordered { .. }))
        .collect();
    assert!(!reordered.is_empty(), "expected reordered fields");
    assert!(reordered.iter().all(|c| c.safety == Safety::Danger));
}

// ─── Pipeline: TypeChangedAndReordered ───────────────────────────────────────

#[test]
fn type_changed_and_reordered_widen_is_danger() {
    let old = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("count",     FieldType::U32),
        ("flag",      FieldType::Bool),
    ]);
    let new = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("flag",      FieldType::Bool),
        ("count",     FieldType::U64), // both reordered and widened
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let changed = classified.iter().find(|c| c.change.name == "count").unwrap();
    assert!(matches!(changed.change.kind, ChangeKind::TypeChangedAndReordered { .. }));
    assert_eq!(changed.safety, Safety::Danger);
}

#[test]
fn type_changed_and_reordered_sign_flip_is_danger() {
    let old = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("count",     FieldType::U64),
        ("flag",      FieldType::Bool),
    ]);
    let new = make_account("Vault", vec![
        ("authority", FieldType::Pubkey),
        ("flag",      FieldType::Bool),
        ("count",     FieldType::I64), // reordered + sign-flipped
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));
    let changed = classified.iter().find(|c| c.change.name == "count").unwrap();
    assert_eq!(changed.safety, Safety::Danger);
}

// ─── Pipeline: mixed changes ──────────────────────────────────────────────────

#[test]
fn mixed_changes_produce_correct_per_field_verdicts() {
    // Simulates a realistic v1→v2 migration with several simultaneous changes
    let old = make_account("Market", vec![
        ("creator",    FieldType::Pubkey),      // 0 — unchanged
        ("balance",    FieldType::U32),         // 1 — widened → Review
        ("is_open",    FieldType::Bool),        // 2 — removed → Danger
        ("bump",       FieldType::U8),          // 3 — unchanged
    ]);
    let new = make_account("Market", vec![
        ("creator",    FieldType::Pubkey),      // 0 — unchanged
        ("balance",    FieldType::U64),         // 1 — widened
        ("bump",       FieldType::U8),          // 2 — was index 3, now 2 → Reordered
        ("created_at", FieldType::I64),         // 3 — new field at end → Safe
    ]);

    let classified = classify_all(diff(&compute_layout(&old), &compute_layout(&new)));

    let by: HashMap<&str, _> = classified.iter().map(|c| (c.change.name.as_str(), c)).collect();

    assert_eq!(by["creator"].safety,    Safety::Safe,   "creator should be unchanged/safe");
    assert_eq!(by["balance"].safety,    Safety::Danger, "balance widen should be Danger — byte layout expands");
    assert_eq!(by["is_open"].safety,    Safety::Danger, "removed field should be Danger");
    assert_eq!(by["bump"].safety,       Safety::Danger, "reordered Borsh field should be Danger — Borsh is positional");
    assert_eq!(by["created_at"].safety, Safety::Danger, "field appended to exact-sized account — old accounts lack bytes, realloc required");
}

// ─── Pipeline: EscrowVault from real IDL — full run ──────────────────────────

#[test]
fn full_pipeline_escrow_vault_v2_with_audit_log_field() {
    let old_def = escrow_vault();

    // v2: add an audit_log pubkey at the end
    let mut new_fields = old_def.fields.clone();
    let idx = new_fields.len();
    new_fields.push(FieldDef { name: "audit_log".to_string(), ty: FieldType::Pubkey, index: idx });
    let new_def = AccountDef { name: old_def.name.clone(), fields: new_fields };

    let changes = diff(&compute_layout(&old_def), &compute_layout(&new_def));
    let classified = classify_all(changes);

    // All existing fields unchanged → Safe
    let existing: Vec<_> = classified.iter()
        .filter(|c| c.change.name != "audit_log")
        .collect();
    assert!(existing.iter().all(|c| c.safety == Safety::Safe && matches!(c.change.kind, ChangeKind::Unchanged)));

    // EscrowVault is exact-sized (227 bytes); audit_log has no bytes in old accounts → DANGER.
    let new_f = classified.iter().find(|c| c.change.name == "audit_log").unwrap();
    assert_eq!(new_f.safety, Safety::Danger);
    assert!(matches!(new_f.change.kind, ChangeKind::Added { .. }));
}

#[test]
fn full_pipeline_escrow_vault_dangerous_mid_removal() {
    let old_def = escrow_vault();

    // Remove total_yes_minted (middle field) — dangerous shift
    let new_def = AccountDef {
        name: old_def.name.clone(),
        fields: reindex(
            old_def.fields.iter().filter(|f| f.name != "total_yes_minted").cloned().collect()
        ),
    };

    let classified = classify_all(diff(
        &compute_layout(&old_def),
        &compute_layout(&new_def),
    ));

    let removed = classified.iter().find(|c| c.change.name == "total_yes_minted").unwrap();
    assert_eq!(removed.safety, Safety::Danger);
    // Fields before the removal are Unchanged (Safe); fields after shift index → Reordered (Danger).
    let unchanged: Vec<_> = classified.iter()
        .filter(|c| c.change.name != "total_yes_minted" && matches!(c.change.kind, ChangeKind::Unchanged))
        .collect();
    assert!(unchanged.iter().all(|c| c.safety == Safety::Safe));
    let reordered: Vec<_> = classified.iter()
        .filter(|c| c.change.name != "total_yes_minted" && matches!(c.change.kind, ChangeKind::Reordered { .. }))
        .collect();
    assert!(!reordered.is_empty(), "fields after removal should be reordered");
    assert!(reordered.iter().all(|c| c.safety == Safety::Danger));
}
