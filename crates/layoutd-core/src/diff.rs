use std::collections::{HashMap, HashSet};

use crate::borsh::{FieldLayout, Layout};
use crate::hints::RenameHint;
use crate::idl::FieldType;

#[derive(Debug, Clone, PartialEq)]
pub enum ChangeKind {
    Unchanged,

    Added { at_index: usize },

    Removed { from_index: usize },

    Renamed { from_name: String },

    TypeChanged {
        old_type: FieldType,
        new_type: FieldType,
    },

    Reordered {
        old_index: usize,
        new_index: usize,
    },

    TypeChangedAndReordered {
        old_type: FieldType,
        old_index: usize,
        new_type: FieldType,
        new_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldChange {
    pub name: String,
    pub kind: ChangeKind,
    /// field info from the old layout; None for newly added fields
    pub old_layout: Option<FieldLayout>,
    /// field info from the new layout; None for removed fields
    pub new_layout: Option<FieldLayout>,
}

/// Diff two layouts using optional rename hints.
///
/// Hints let callers force a rename detection for cases the automatic
/// inference misses: ambiguous same-type swaps and rename+reorder combos.
/// A hint is honoured only when `from` and `to` share the same type;
/// type-mismatched hints are silently ignored (Remove+Add per spec).
pub fn diff_with_hints(old: &Layout, new: &Layout, hints: &[RenameHint]) -> Vec<FieldChange> {
    let account = old.account_name.as_str();
    let account_hints: Vec<&RenameHint> =
        hints.iter().filter(|h| h.account == account).collect();
    diff_inner(old, new, &account_hints)
}

/// Diff two layouts with no hints (backward-compatible entry point).
pub fn diff(old: &Layout, new: &Layout) -> Vec<FieldChange> {
    diff_inner(old, new, &[])
}

fn diff_inner(old: &Layout, new: &Layout, hints: &[&RenameHint]) -> Vec<FieldChange> {
    let old_map: HashMap<&str, &FieldLayout> =
        old.fields.iter().map(|f| (f.name.as_str(), f)).collect();
    let new_map: HashMap<&str, &FieldLayout> =
        new.fields.iter().map(|f| (f.name.as_str(), f)).collect();

    let mut matched: Vec<FieldChange> = Vec::new();
    let mut removed: Vec<FieldChange> = Vec::new();
    let mut added: Vec<FieldChange> = Vec::new();

    for old_field in &old.fields {
        if let Some(new_field) = new_map.get(old_field.name.as_str()) {
            matched.push(FieldChange {
                name: old_field.name.clone(),
                kind: classify_change(old_field, new_field),
                old_layout: Some(old_field.clone()),
                new_layout: Some((*new_field).clone()),
            });
        } else {
            removed.push(FieldChange {
                name: old_field.name.clone(),
                kind: ChangeKind::Removed { from_index: old_field.index },
                old_layout: Some(old_field.clone()),
                new_layout: None,
            });
        }
    }

    for new_field in &new.fields {
        if !old_map.contains_key(new_field.name.as_str()) {
            added.push(FieldChange {
                name: new_field.name.clone(),
                kind: ChangeKind::Added { at_index: new_field.index },
                old_layout: None,
                new_layout: Some(new_field.clone()),
            });
        }
    }

    // ── Phase 1: hint-forced renames ─────────────────────────────────────────
    // Apply explicit hints first; they override auto-inference for the matched
    // slots so that ambiguous cases (same-type swap, rename+reorder) are resolved.
    let mut hint_pairs: Vec<(usize, usize)> = Vec::new();
    for hint in hints {
        let r_idx = removed.iter().position(|r| r.name == hint.from);
        let a_idx = added.iter().position(|a| a.name == hint.to);
        if let (Some(r_idx), Some(a_idx)) = (r_idx, a_idx) {
            let r_ty = &removed[r_idx].old_layout.as_ref().unwrap().ty;
            let a_ty = &added[a_idx].new_layout.as_ref().unwrap().ty;
            // Only honour the hint when types match; type mismatch → Remove+Add per spec.
            if r_ty == a_ty {
                hint_pairs.push((r_idx, a_idx));
            }
        }
    }
    let hint_r: HashSet<usize> = hint_pairs.iter().map(|(r, _)| *r).collect();
    let hint_a: HashSet<usize> = hint_pairs.iter().map(|(_, a)| *a).collect();

    // ── Phase 2: auto rename inference (on fields not claimed by a hint) ──────
    // A removed field and an added field at the same index with the same type
    // is a rename. Only commit when exactly one added field matches each removed.
    let mut rename_pairs: Vec<(usize, usize)> = Vec::new();

    for (r_idx, r) in removed.iter().enumerate() {
        if hint_r.contains(&r_idx) {
            continue; // already claimed by a hint
        }
        let r_fl = r.old_layout.as_ref().unwrap();
        let candidates: Vec<usize> = added
            .iter()
            .enumerate()
            .filter(|(a_idx, a)| {
                if hint_a.contains(a_idx) {
                    return false;
                }
                let a_fl = a.new_layout.as_ref().unwrap();
                a_fl.index == r_fl.index && a_fl.ty == r_fl.ty
            })
            .map(|(i, _)| i)
            .collect();
        if candidates.len() == 1 {
            rename_pairs.push((r_idx, candidates[0]));
        }
    }

    // If two removed fields somehow claimed the same added slot, cancel both.
    let mut added_claim_count: HashMap<usize, usize> = HashMap::new();
    for (_, a_idx) in &rename_pairs {
        *added_claim_count.entry(*a_idx).or_insert(0) += 1;
    }
    let valid_auto_renames: Vec<(usize, usize)> = rename_pairs
        .into_iter()
        .filter(|(_, a_idx)| added_claim_count[a_idx] == 1)
        .collect();

    // ── Merge hint renames + auto renames ─────────────────────────────────────
    let all_renames: Vec<(usize, usize)> = hint_pairs
        .iter()
        .chain(valid_auto_renames.iter())
        .copied()
        .collect();

    let renamed_r: HashSet<usize> = all_renames.iter().map(|(r, _)| *r).collect();
    let renamed_a: HashSet<usize> = all_renames.iter().map(|(_, a)| *a).collect();

    let mut changes: Vec<FieldChange> = Vec::new();

    for (r_idx, a_idx) in &all_renames {
        let old_fl = removed[*r_idx].old_layout.as_ref().unwrap();
        let new_fl = added[*a_idx].new_layout.as_ref().unwrap();
        changes.push(FieldChange {
            name: new_fl.name.clone(),
            kind: ChangeKind::Renamed { from_name: old_fl.name.clone() },
            old_layout: Some(old_fl.clone()),
            new_layout: Some(new_fl.clone()),
        });
    }

    for (r_idx, r) in removed.into_iter().enumerate() {
        if !renamed_r.contains(&r_idx) {
            changes.push(r);
        }
    }

    for (a_idx, a) in added.into_iter().enumerate() {
        if !renamed_a.contains(&a_idx) {
            changes.push(a);
        }
    }

    changes.extend(matched);

    // Sort by old index first (for removed/renamed/matched), new index for added-only fields.
    changes.sort_by_key(|c| match &c.kind {
        ChangeKind::Removed { from_index } => *from_index,
        _ => c.new_layout.as_ref().expect("non-removed change must have new_layout").index,
    });

    changes
}

fn classify_change(old: &FieldLayout, new: &FieldLayout) -> ChangeKind {
    let type_changed = old.ty != new.ty;
    let index_changed = old.index != new.index;

    match (type_changed, index_changed) {
        (false, false) => ChangeKind::Unchanged,
        (true, false) => ChangeKind::TypeChanged {
            old_type: old.ty.clone(),
            new_type: new.ty.clone(),
        },
        (false, true) => ChangeKind::Reordered {
            old_index: old.index,
            new_index: new.index,
        },
        (true, true) => ChangeKind::TypeChangedAndReordered {
            old_type: old.ty.clone(),
            old_index: old.index,
            new_type: new.ty.clone(),
            new_index: new.index,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::borsh::compute_layout;
    use crate::idl::{AccountDef, FieldDef, FieldType};

    fn make_layout(name: &str, fields: Vec<(&str, FieldType)>) -> Layout {
        let def = AccountDef {
            name: name.to_string(),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(i, (n, ty))| FieldDef { name: n.to_string(), ty, index: i })
                .collect(),
        };
        compute_layout(&def)
    }

    #[test]
    fn unchanged() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
        ]);
        let changes = diff(&old, &new);
        assert!(changes.iter().all(|c| c.kind == ChangeKind::Unchanged));
    }

    #[test]
    fn field_added_at_end() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
            ("bump",      FieldType::U8),
        ]);
        let changes = diff(&old, &new);
        let added: Vec<_> = changes.iter()
            .filter(|c| matches!(c.kind, ChangeKind::Added { .. }))
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].name, "bump");
    }

    #[test]
    fn field_removed() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
            ("bump",      FieldType::U8),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
        ]);
        let changes = diff(&old, &new);
        let removed: Vec<_> = changes.iter()
            .filter(|c| matches!(c.kind, ChangeKind::Removed { .. }))
            .collect();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name, "bump");
    }

    #[test]
    fn type_changed() {
        let old = make_layout("Vault", vec![("balance", FieldType::U32)]);
        let new = make_layout("Vault", vec![("balance", FieldType::U64)]);
        let changes = diff(&old, &new);
        assert!(matches!(changes[0].kind, ChangeKind::TypeChanged { .. }));
    }

    #[test]
    fn field_reordered() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("bump",      FieldType::U8),
            ("balance",   FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
            ("bump",      FieldType::U8),
        ]);
        let changes = diff(&old, &new);
        let reordered: Vec<_> = changes.iter()
            .filter(|c| matches!(c.kind, ChangeKind::Reordered { .. }))
            .collect();
        assert!(!reordered.is_empty());
    }

    #[test]
    fn rename_detected_when_same_type_and_index() {
        let old = make_layout("Vault", vec![
            ("owner",  FieldType::Pubkey),
            ("amount", FieldType::U64),
            ("bump",   FieldType::U8),
        ]);
        let new = make_layout("Vault", vec![
            ("owner",   FieldType::Pubkey),
            ("balance", FieldType::U64), // renamed from amount, same type+index
            ("bump",    FieldType::U8),
        ]);
        let changes = diff(&old, &new);
        let renamed = changes.iter().find(|c| matches!(c.kind, ChangeKind::Renamed { .. }));
        assert!(renamed.is_some(), "expected a rename to be detected");
        let r = renamed.unwrap();
        assert_eq!(r.name, "balance");
        assert!(matches!(&r.kind, ChangeKind::Renamed { from_name } if from_name == "amount"));
    }

    #[test]
    fn rename_not_detected_when_types_differ() {
        // Different type → not a rename, must be Removed + Added
        let old = make_layout("Vault", vec![
            ("owner",  FieldType::Pubkey),
            ("amount", FieldType::U32),
            ("bump",   FieldType::U8),
        ]);
        let new = make_layout("Vault", vec![
            ("owner",   FieldType::Pubkey),
            ("balance", FieldType::U64), // different type — not a rename
            ("bump",    FieldType::U8),
        ]);
        let changes = diff(&old, &new);
        assert!(!changes.iter().any(|c| matches!(c.kind, ChangeKind::Renamed { .. })));
        assert!(changes.iter().any(|c| c.name == "amount" && matches!(c.kind, ChangeKind::Removed { .. })));
        assert!(changes.iter().any(|c| c.name == "balance" && matches!(c.kind, ChangeKind::Added { .. })));
    }

    #[test]
    fn multiple_simultaneous_renames_detected() {
        let old = make_layout("Vault", vec![
            ("alpha", FieldType::Pubkey),
            ("beta",  FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("gamma", FieldType::Pubkey), // renamed from alpha
            ("delta", FieldType::U64),    // renamed from beta
        ]);
        let changes = diff(&old, &new);
        let renames: Vec<_> = changes.iter()
            .filter(|c| matches!(c.kind, ChangeKind::Renamed { .. }))
            .collect();
        assert_eq!(renames.len(), 2);
    }

    #[test]
    fn type_changed_and_reordered_detected() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("count",     FieldType::U32),
            ("flag",      FieldType::Bool),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("flag",      FieldType::Bool),
            ("count",     FieldType::U64), // both reordered and type widened
        ]);
        let changes = diff(&old, &new);
        let tc_r = changes.iter().find(|c| c.name == "count").unwrap();
        assert!(matches!(tc_r.kind, ChangeKind::TypeChangedAndReordered { .. }));
    }

    // ── hint-based rename tests ────────────────────────────────────────────────

    fn hint(account: &str, from: &str, to: &str) -> RenameHint {
        RenameHint { account: account.to_string(), from: from.to_string(), to: to.to_string() }
    }

    #[test]
    fn hint_forces_rename_when_auto_would_miss_due_to_position_change() {
        // alpha at index 0 renamed to beta at index 2 — positions differ so auto-detection misses it.
        let old = make_layout("Vault", vec![
            ("alpha", FieldType::U64),
            ("mid",   FieldType::U32),
        ]);
        let new = make_layout("Vault", vec![
            ("mid",  FieldType::U32),
            ("beta", FieldType::U64), // renamed from alpha, moved to end
        ]);
        let hints = [hint("Vault", "alpha", "beta")];
        let changes = diff_with_hints(&old, &new, &hints);
        let renamed = changes.iter().find(|c| c.name == "beta");
        assert!(
            matches!(renamed.map(|c| &c.kind), Some(ChangeKind::Renamed { from_name }) if from_name == "alpha"),
            "expected Renamed {{from_name: alpha}}, changes: {:?}", changes
        );
    }

    #[test]
    fn hint_forces_rename_when_reorder_masks_it() {
        // alpha (index 0) is renamed to gamma AND moved to index 2 simultaneously.
        // Without a hint the auto-detection requires same index, so it sees only Remove+Add.
        // With a hint it correctly produces a Renamed result.
        let old = make_layout("Vault", vec![
            ("alpha", FieldType::U64), // index 0
            ("beta",  FieldType::U32), // index 1
            ("omega", FieldType::U8),  // index 2
        ]);
        let new = make_layout("Vault", vec![
            ("beta",  FieldType::U32), // index 0 (moved)
            ("omega", FieldType::U8),  // index 1 (moved)
            ("gamma", FieldType::U64), // index 2 (was alpha at 0, renamed)
        ]);

        // Without hints: alpha→removed, gamma→added (indices differ, auto won't pair them).
        let no_hint = diff(&old, &new);
        assert!(
            no_hint.iter().any(|c| c.name == "alpha" && matches!(c.kind, ChangeKind::Removed { .. })),
            "without hints, alpha should appear as Removed"
        );
        assert!(
            no_hint.iter().any(|c| c.name == "gamma" && matches!(c.kind, ChangeKind::Added { .. })),
            "without hints, gamma should appear as Added"
        );
        assert!(
            !no_hint.iter().any(|c| matches!(c.kind, ChangeKind::Renamed { .. })),
            "without hints, no rename should be detected for cross-position rename"
        );

        // With a hint: alpha → gamma is a forced rename.
        let hints = [hint("Vault", "alpha", "gamma")];
        let hinted = diff_with_hints(&old, &new, &hints);
        let renamed = hinted.iter().find(|c| c.name == "gamma");
        assert!(
            matches!(renamed.map(|c| &c.kind), Some(ChangeKind::Renamed { from_name }) if from_name == "alpha"),
            "with hint, alpha→gamma should be Renamed"
        );
        // alpha and gamma must not appear as Remove/Add anymore
        assert!(!hinted.iter().any(|c| c.name == "alpha" && matches!(c.kind, ChangeKind::Removed { .. })));
        assert!(!hinted.iter().any(|c| c.name == "gamma" && matches!(c.kind, ChangeKind::Added { .. })));
    }

    #[test]
    fn hint_with_type_mismatch_is_ignored() {
        // Hint says rename alpha→beta, but types differ → ignore hint → Remove + Add.
        let old = make_layout("Vault", vec![("alpha", FieldType::U32)]);
        let new = make_layout("Vault", vec![("beta",  FieldType::U64)]);
        let hints = [hint("Vault", "alpha", "beta")];
        let changes = diff_with_hints(&old, &new, &hints);
        assert!(changes.iter().any(|c| c.name == "alpha" && matches!(c.kind, ChangeKind::Removed { .. })));
        assert!(changes.iter().any(|c| c.name == "beta"  && matches!(c.kind, ChangeKind::Added { .. })));
        assert!(!changes.iter().any(|c| matches!(c.kind, ChangeKind::Renamed { .. })));
    }

    #[test]
    fn hint_for_wrong_account_is_ignored() {
        let old = make_layout("Vault", vec![("alpha", FieldType::U64)]);
        let new = make_layout("Vault", vec![("beta",  FieldType::U64)]);
        let hints = [hint("Market", "alpha", "beta")]; // wrong account
        let changes = diff_with_hints(&old, &new, &hints);
        // auto-detection: same type, same index 0 → rename IS detected automatically
        // (the hint is for a different account, so auto runs freely)
        let renamed = changes.iter().find(|c| matches!(c.kind, ChangeKind::Renamed { .. }));
        assert!(renamed.is_some(), "auto-detection should still find the rename");
    }

    #[test]
    fn empty_hints_behaves_identically_to_plain_diff() {
        let old = make_layout("Vault", vec![
            ("owner",  FieldType::Pubkey),
            ("amount", FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("owner",   FieldType::Pubkey),
            ("balance", FieldType::U64), // auto-detectable rename
        ]);
        let plain  = diff(&old, &new);
        let hinted = diff_with_hints(&old, &new, &[]);
        assert_eq!(plain, hinted);
    }
}
