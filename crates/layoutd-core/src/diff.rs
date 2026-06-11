use std::collections::{HashMap, HashSet};

use crate::borsh::{FieldLayout, Layout};
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

pub fn diff(old: &Layout, new: &Layout) -> Vec<FieldChange> {
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

    // Rename inference: a removed field and an added field at the same index with the same type
    // is a rename. Only commit when exactly one added field matches each removed field.
    let mut rename_pairs: Vec<(usize, usize)> = Vec::new(); // (removed_idx, added_idx)

    for (r_idx, r) in removed.iter().enumerate() {
        let r_fl = r.old_layout.as_ref().unwrap();
        let candidates: Vec<usize> = added
            .iter()
            .enumerate()
            .filter(|(_, a)| {
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
    let valid_renames: Vec<(usize, usize)> = rename_pairs
        .into_iter()
        .filter(|(_, a_idx)| added_claim_count[a_idx] == 1)
        .collect();

    let renamed_r: HashSet<usize> = valid_renames.iter().map(|(r, _)| *r).collect();
    let renamed_a: HashSet<usize> = valid_renames.iter().map(|(_, a)| *a).collect();

    let mut changes: Vec<FieldChange> = Vec::new();

    for (r_idx, a_idx) in &valid_renames {
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
}
