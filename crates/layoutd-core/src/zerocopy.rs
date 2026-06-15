use crate::borsh::{FieldLayout, Layout, Offset, Size};
use crate::classify::{ClassifiedChange, Safety};
use crate::diff::{ChangeKind, FieldChange};
use crate::idl::{AccountDef, FieldType};

// ── data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ZcFieldLayout {
    pub name: String,
    pub ty: FieldType,
    pub index: usize,
    /// Absolute byte offset in account data (includes the 8-byte discriminator prefix).
    pub offset: usize,
    pub size: usize,
    pub align: usize,
    /// Padding bytes inserted *before* this field to satisfy its alignment.
    pub padding_before: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZcLayout {
    pub account_name: String,
    pub fields: Vec<ZcFieldLayout>,
    /// Total account data size (discriminator + struct body + trailing padding).
    pub total_size: usize,
    /// Alignment of the struct = max alignment across all fields.
    pub struct_align: usize,
    /// Bytes appended after the last field so total_size % struct_align == 0.
    pub trailing_padding: usize,
}

// ── alignment / size rules ────────────────────────────────────────────────────

/// `repr(C)` alignment of a type.  Returns `None` for types that are not
/// valid in a zero-copy (Pod / repr(C)) struct.
pub fn align_of(ty: &FieldType) -> Option<usize> {
    match ty {
        FieldType::U8 | FieldType::I8 | FieldType::Bool => Some(1),
        FieldType::U16 | FieldType::I16 => Some(2),
        FieldType::U32 | FieldType::I32 | FieldType::F32 => Some(4),
        FieldType::U64 | FieldType::I64 | FieldType::F64 => Some(8),
        FieldType::U128 | FieldType::I128 => Some(16),
        // Pubkey is a newtype around [u8; 32] — byte-array, alignment 1.
        FieldType::Pubkey => Some(1),
        // Array<T, N>: alignment equals alignment of T.
        FieldType::Array(inner, _) => align_of(inner),
        // Variable-size / opaque types are not allowed in zero-copy structs.
        FieldType::String
        | FieldType::Vec(_)
        | FieldType::Option(_)
        | FieldType::Defined(_)
        | FieldType::Unknown(_) => None,
    }
}

/// Size in bytes of a type in a `repr(C)` struct.  Returns `None` for types
/// that are not valid in a zero-copy struct.
pub fn zc_size_of(ty: &FieldType) -> Option<usize> {
    match ty {
        FieldType::U8 | FieldType::I8 | FieldType::Bool => Some(1),
        FieldType::U16 | FieldType::I16 => Some(2),
        FieldType::U32 | FieldType::I32 | FieldType::F32 => Some(4),
        FieldType::U64 | FieldType::I64 | FieldType::F64 => Some(8),
        FieldType::U128 | FieldType::I128 => Some(16),
        FieldType::Pubkey => Some(32),
        FieldType::Array(inner, n) => zc_size_of(inner).map(|s| s * n),
        FieldType::String
        | FieldType::Vec(_)
        | FieldType::Option(_)
        | FieldType::Defined(_)
        | FieldType::Unknown(_) => None,
    }
}

#[inline]
fn next_aligned(offset: usize, align: usize) -> usize {
    (offset + align - 1) / align * align
}

// ── layout engine ─────────────────────────────────────────────────────────────

/// Compute the exact `repr(C)` byte layout of a zero-copy account.
///
/// Returns `Err` if any field has a type that is not valid in a `repr(C)` struct
/// (e.g. `String`, `Vec`, `Option`, user-defined types).
pub fn compute_zc_layout(def: &AccountDef) -> Result<ZcLayout, String> {
    let mut struct_offset: usize = 0; // offset within the struct body (after discriminator)
    let mut struct_align: usize = 1;
    let mut fields = Vec::with_capacity(def.fields.len());

    for field in &def.fields {
        let a = align_of(&field.ty).ok_or_else(|| {
            format!(
                "field '{}' has type that is not valid in a zero-copy (repr(C)) struct: {:?}",
                field.name, field.ty
            )
        })?;
        let s = zc_size_of(&field.ty).ok_or_else(|| {
            format!(
                "field '{}' has unsized type — not allowed in zero-copy structs",
                field.name
            )
        })?;

        struct_align = struct_align.max(a);
        let aligned = next_aligned(struct_offset, a);
        let padding_before = aligned - struct_offset;

        fields.push(ZcFieldLayout {
            name: field.name.clone(),
            ty: field.ty.clone(),
            index: field.index,
            offset: 8 + aligned, // absolute offset: 8-byte discriminator prefix
            size: s,
            align: a,
            padding_before,
        });

        struct_offset = aligned + s;
    }

    let padded_struct = next_aligned(struct_offset, struct_align);
    let trailing_padding = padded_struct - struct_offset;
    let total_size = 8 + padded_struct;

    Ok(ZcLayout {
        account_name: def.name.clone(),
        fields,
        total_size,
        struct_align,
        trailing_padding,
    })
}

/// Convert a `ZcLayout` to the borsh `Layout` type so the existing `diff()`
/// engine can be reused without modification.  All offsets are exact `Fixed`
/// values; there are no `AfterVariable` offsets in a zero-copy struct.
pub fn zc_to_borsh_layout(zc: &ZcLayout) -> Layout {
    let fields = zc
        .fields
        .iter()
        .map(|f| FieldLayout {
            name: f.name.clone(),
            ty: f.ty.clone(),
            index: f.index,
            offset: Offset::Fixed(f.offset),
            size: Size::Fixed(f.size),
        })
        .collect();

    Layout {
        account_name: zc.account_name.clone(),
        fields,
        total_size: Size::Fixed(zc.total_size),
    }
}

// ── zero-copy classifier ──────────────────────────────────────────────────────

/// Classify a change list under `repr(C)` / zero-copy rules.
///
/// This is stricter than the Borsh classifier:
/// - **Reordered** → Danger (declaration order controls byte offsets)
/// - **Added in middle** → Danger (shifts all following fields)
/// - **TypeChanged with different size or alignment** → Danger
/// - **TypeChanged with same size and alignment** → Review
pub fn classify_zc_all(
    changes: Vec<FieldChange>,
    _old: &ZcLayout,
    _new: &ZcLayout,
) -> Vec<ClassifiedChange> {
    let max_new_index = changes
        .iter()
        .filter_map(|c| c.new_layout.as_ref())
        .map(|fl| fl.index)
        .max()
        .unwrap_or(0);

    changes
        .into_iter()
        .map(|c| classify_zc_one(c, max_new_index))
        .collect()
}

fn classify_zc_one(change: FieldChange, max_new_index: usize) -> ClassifiedChange {
    let (safety, reason) = match &change.kind {
        ChangeKind::Unchanged => (Safety::Safe, "field unchanged"),

        ChangeKind::Renamed { .. } => (
            Safety::Safe,
            "field renamed — same type and position, byte offset is identical",
        ),

        ChangeKind::Added { at_index } => {
            if *at_index >= max_new_index {
                (
                    Safety::Safe,
                    "field added at end — no existing field offsets change",
                )
            } else {
                (
                    Safety::Danger,
                    "field inserted in middle — shifts byte offsets of all following fields, \
                     corrupting zero-copy reads on existing accounts",
                )
            }
        }

        ChangeKind::Removed { .. } => (
            Safety::Danger,
            "field removed — permanent data loss and byte-offset shift for all following fields",
        ),

        ChangeKind::Reordered { .. } => (
            Safety::Danger,
            "field reordered — zero-copy uses declaration order for offsets; \
             reordering changes every affected field's byte address",
        ),

        ChangeKind::TypeChanged { old_type, new_type } => {
            classify_zc_type_change(old_type, new_type)
        }

        ChangeKind::TypeChangedAndReordered { old_type, new_type, .. } => {
            let (tc_safety, tc_reason) = classify_zc_type_change(old_type, new_type);
            if tc_safety == Safety::Danger {
                (Safety::Danger, tc_reason)
            } else {
                (
                    Safety::Danger,
                    "field reordered and type changed — both byte offset and bit interpretation differ",
                )
            }
        }
    };

    ClassifiedChange { change, safety, reason }
}

fn classify_zc_type_change(old_ty: &FieldType, new_ty: &FieldType) -> (Safety, &'static str) {
    let old_size = zc_size_of(old_ty);
    let new_size = zc_size_of(new_ty);
    let old_align = align_of(old_ty);
    let new_align = align_of(new_ty);

    if old_size != new_size {
        return (
            Safety::Danger,
            "type size changed — shifts byte offsets of all following fields in zero-copy struct",
        );
    }

    if old_align != new_align {
        return (
            Safety::Danger,
            "type alignment changed — field offset and struct padding may change, \
             corrupting the zero-copy layout",
        );
    }

    // Same size and alignment: this field's slot in memory is unchanged.
    // The bits are reinterpreted with a different Rust type — semantically Review.
    (
        Safety::Review,
        "type changed but size and alignment unchanged — byte offset preserved; \
         verify semantic correctness of the bit reinterpretation",
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff;
    use crate::idl::{AccountDef, FieldDef};

    fn def(fields: Vec<(&str, FieldType)>) -> AccountDef {
        AccountDef {
            name: "TestAccount".to_string(),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(i, (n, ty))| FieldDef {
                    name: n.to_string(),
                    ty,
                    index: i,
                })
                .collect(),
        }
    }

    // ── align_of / zc_size_of ─────────────────────────────────────────────────

    #[test]
    fn align_and_size_primitives() {
        assert_eq!(align_of(&FieldType::U8), Some(1));
        assert_eq!(align_of(&FieldType::U16), Some(2));
        assert_eq!(align_of(&FieldType::U32), Some(4));
        assert_eq!(align_of(&FieldType::U64), Some(8));
        assert_eq!(align_of(&FieldType::U128), Some(16));
        assert_eq!(align_of(&FieldType::Bool), Some(1));
        assert_eq!(align_of(&FieldType::F32), Some(4));
        assert_eq!(align_of(&FieldType::F64), Some(8));
        assert_eq!(align_of(&FieldType::Pubkey), Some(1));

        assert_eq!(zc_size_of(&FieldType::U8), Some(1));
        assert_eq!(zc_size_of(&FieldType::U128), Some(16));
        assert_eq!(zc_size_of(&FieldType::Pubkey), Some(32));
    }

    #[test]
    fn align_and_size_array() {
        let ty = FieldType::Array(Box::new(FieldType::U64), 4);
        assert_eq!(align_of(&ty), Some(8));
        assert_eq!(zc_size_of(&ty), Some(32));
    }

    #[test]
    fn align_of_variable_types_is_none() {
        assert_eq!(align_of(&FieldType::String), None);
        assert_eq!(align_of(&FieldType::Vec(Box::new(FieldType::U8))), None);
        assert_eq!(align_of(&FieldType::Option(Box::new(FieldType::U64))), None);
        assert_eq!(align_of(&FieldType::Defined("Foo".into())), None);
    }

    // ── layout engine ─────────────────────────────────────────────────────────

    #[test]
    fn single_u64_starts_at_offset_8() {
        let zc = compute_zc_layout(&def(vec![("amount", FieldType::U64)])).unwrap();
        let f = &zc.fields[0];
        assert_eq!(f.offset, 8);
        assert_eq!(f.size, 8);
        assert_eq!(f.align, 8);
        assert_eq!(f.padding_before, 0);
    }

    #[test]
    fn u8_then_u64_inserts_padding() {
        // struct { x: u8, y: u64 }
        // x at struct[0], y must be 8-byte aligned → struct[8]
        // padding_before y = 7
        let zc = compute_zc_layout(&def(vec![
            ("x", FieldType::U8),
            ("y", FieldType::U64),
        ]))
        .unwrap();

        assert_eq!(zc.fields[0].offset, 8);          // absolute: disc(8) + struct[0]
        assert_eq!(zc.fields[0].padding_before, 0);

        assert_eq!(zc.fields[1].offset, 16);          // absolute: disc(8) + struct[8]
        assert_eq!(zc.fields[1].padding_before, 7);

        assert_eq!(zc.struct_align, 8);
        // struct body = 1(x) + 7(pad) + 8(y) = 16, no trailing padding
        assert_eq!(zc.trailing_padding, 0);
        assert_eq!(zc.total_size, 24);
    }

    #[test]
    fn u64_then_u8_adds_trailing_padding() {
        // struct { x: u64, y: u8 }
        // x at struct[0], y at struct[8], struct body = 9 bytes
        // struct_align = 8 → pad to 16 → trailing_padding = 7
        let zc = compute_zc_layout(&def(vec![
            ("x", FieldType::U64),
            ("y", FieldType::U8),
        ]))
        .unwrap();

        assert_eq!(zc.fields[0].offset, 8);
        assert_eq!(zc.fields[1].offset, 16);
        assert_eq!(zc.fields[1].padding_before, 0);
        assert_eq!(zc.trailing_padding, 7);
        assert_eq!(zc.total_size, 24);
    }

    #[test]
    fn cascading_alignment_u8_u16_u32() {
        // struct { a: u8, b: u16, c: u32 }
        // a at struct[0], b needs align 2 → struct[2], padding=1
        // c needs align 4 → struct[4], padding=0
        // struct body = 8, struct_align = 4, 8 % 4 == 0 → no trailing
        let zc = compute_zc_layout(&def(vec![
            ("a", FieldType::U8),
            ("b", FieldType::U16),
            ("c", FieldType::U32),
        ]))
        .unwrap();

        assert_eq!(zc.fields[0].offset, 8);   // 8 + 0
        assert_eq!(zc.fields[1].offset, 10);  // 8 + 2
        assert_eq!(zc.fields[1].padding_before, 1);
        assert_eq!(zc.fields[2].offset, 12);  // 8 + 4
        assert_eq!(zc.fields[2].padding_before, 0);
        assert_eq!(zc.struct_align, 4);
        assert_eq!(zc.trailing_padding, 0);
        // struct body: 1(a) + 1(pad) + 2(b) + 4(c) = 8 bytes; total = disc(8) + 8 = 16
        assert_eq!(zc.total_size, 16);
    }

    #[test]
    fn pubkey_has_align_1_size_32() {
        let zc = compute_zc_layout(&def(vec![
            ("owner", FieldType::Pubkey),
            ("bump", FieldType::U8),
        ]))
        .unwrap();

        assert_eq!(zc.fields[0].offset, 8);   // disc + 0
        assert_eq!(zc.fields[0].size, 32);
        assert_eq!(zc.fields[0].align, 1);
        assert_eq!(zc.fields[1].offset, 40);  // disc + 32
        assert_eq!(zc.fields[1].padding_before, 0);
        // struct_align = max(1,1) = 1, trailing = 0
        assert_eq!(zc.struct_align, 1);
        assert_eq!(zc.trailing_padding, 0);
        assert_eq!(zc.total_size, 41);
    }

    #[test]
    fn u128_field_align_16() {
        // u128 at struct[0], but disc is 8 bytes so struct body starts at offset 8.
        // For alignment within the struct body: struct[0] is fine.
        let zc = compute_zc_layout(&def(vec![("val", FieldType::U128)])).unwrap();
        assert_eq!(zc.fields[0].offset, 8);
        assert_eq!(zc.fields[0].align, 16);
        assert_eq!(zc.fields[0].padding_before, 0);
        assert_eq!(zc.struct_align, 16);
        assert_eq!(zc.total_size, 24); // 8 + 16
    }

    #[test]
    fn u8_then_u128_padding_before_is_15() {
        // struct { x: u8, y: u128 }
        // x at struct[0], y needs align 16 → struct[16], padding_before = 15
        let zc = compute_zc_layout(&def(vec![
            ("x", FieldType::U8),
            ("y", FieldType::U128),
        ]))
        .unwrap();

        assert_eq!(zc.fields[0].offset, 8);   // abs
        assert_eq!(zc.fields[1].offset, 24);  // disc(8) + struct[16]
        assert_eq!(zc.fields[1].padding_before, 15);
        assert_eq!(zc.struct_align, 16);
        assert_eq!(zc.trailing_padding, 0); // 1+15+16=32, 32%16==0
        assert_eq!(zc.total_size, 40);
    }

    #[test]
    fn array_of_u64_correct_layout() {
        // [u64; 4]: align=8, size=32
        let ty = FieldType::Array(Box::new(FieldType::U64), 4);
        let zc = compute_zc_layout(&def(vec![("keys", ty)])).unwrap();
        assert_eq!(zc.fields[0].offset, 8);
        assert_eq!(zc.fields[0].size, 32);
        assert_eq!(zc.fields[0].align, 8);
        assert_eq!(zc.total_size, 40);
    }

    #[test]
    fn variable_type_returns_error() {
        let err = compute_zc_layout(&def(vec![
            ("owner", FieldType::Pubkey),
            ("label", FieldType::String),
        ]))
        .unwrap_err();
        assert!(err.contains("label"), "error should name the bad field: {err}");
    }

    #[test]
    fn defined_type_returns_error() {
        let err =
            compute_zc_layout(&def(vec![("state", FieldType::Defined("MarketState".into()))]))
                .unwrap_err();
        assert!(err.contains("state"), "{err}");
    }

    // ── vault layout cross-check ──────────────────────────────────────────────

    #[test]
    fn vault_v1_layout_offsets() {
        // owner(Pubkey,32,align=1), balance(u64,8,align=8), bump(u8,1,align=1)
        // struct[0..32] = owner, struct[32..40] = balance (32%8==0, no pad), struct[40] = bump
        // struct_align = 8, struct body = 41, trailing = 7, total = 8+48 = 56
        let zc = compute_zc_layout(&def(vec![
            ("owner", FieldType::Pubkey),
            ("balance", FieldType::U64),
            ("bump", FieldType::U8),
        ]))
        .unwrap();

        assert_eq!(zc.fields[0].offset, 8);   // owner
        assert_eq!(zc.fields[1].offset, 40);  // balance: disc(8)+struct(32)
        assert_eq!(zc.fields[1].padding_before, 0);
        assert_eq!(zc.fields[2].offset, 48);  // bump
        assert_eq!(zc.struct_align, 8);
        assert_eq!(zc.trailing_padding, 7);
        assert_eq!(zc.total_size, 56);
    }

    // ── zc_to_borsh_layout ────────────────────────────────────────────────────

    #[test]
    fn zc_to_borsh_preserves_offsets() {
        let zc = compute_zc_layout(&def(vec![
            ("x", FieldType::U8),
            ("y", FieldType::U64),
        ]))
        .unwrap();

        let borsh = zc_to_borsh_layout(&zc);
        assert_eq!(borsh.fields[0].offset, Offset::Fixed(8));
        assert_eq!(borsh.fields[1].offset, Offset::Fixed(16));
        assert_eq!(borsh.total_size, Size::Fixed(24));
    }

    // ── ZC classifier ─────────────────────────────────────────────────────────

    fn make_zc(fields: Vec<(&str, FieldType)>) -> ZcLayout {
        compute_zc_layout(&def(fields)).unwrap()
    }

    fn zc_changes(
        old_fields: Vec<(&str, FieldType)>,
        new_fields: Vec<(&str, FieldType)>,
    ) -> Vec<ClassifiedChange> {
        let old_zc = make_zc(old_fields);
        let new_zc = make_zc(new_fields);
        let changes = diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc));
        classify_zc_all(changes, &old_zc, &new_zc)
    }

    #[test]
    fn unchanged_is_safe_zc() {
        let cs = zc_changes(
            vec![("owner", FieldType::Pubkey), ("balance", FieldType::U64)],
            vec![("owner", FieldType::Pubkey), ("balance", FieldType::U64)],
        );
        assert!(cs.iter().all(|c| c.safety == Safety::Safe));
    }

    #[test]
    fn add_field_at_end_is_safe_zc() {
        let cs = zc_changes(
            vec![("owner", FieldType::Pubkey), ("balance", FieldType::U64)],
            vec![
                ("owner", FieldType::Pubkey),
                ("balance", FieldType::U64),
                ("bump", FieldType::U8),
            ],
        );
        let added = cs.iter().find(|c| c.change.name == "bump").unwrap();
        assert_eq!(added.safety, Safety::Safe);
    }

    #[test]
    fn add_field_in_middle_is_danger_zc() {
        // In Borsh this is Review; in zero-copy it's Danger.
        let cs = zc_changes(
            vec![("owner", FieldType::Pubkey), ("bump", FieldType::U8)],
            vec![
                ("owner", FieldType::Pubkey),
                ("balance", FieldType::U64), // inserted before bump
                ("bump", FieldType::U8),
            ],
        );
        let added = cs.iter().find(|c| c.change.name == "balance").unwrap();
        assert_eq!(added.safety, Safety::Danger);
    }

    #[test]
    fn reorder_is_danger_zc() {
        // In Borsh reorder is Safe; in zero-copy it's Danger.
        let cs = zc_changes(
            vec![
                ("owner", FieldType::Pubkey),
                ("balance", FieldType::U64),
                ("bump", FieldType::U8),
            ],
            vec![
                ("owner", FieldType::Pubkey),
                ("bump", FieldType::U8),   // swapped
                ("balance", FieldType::U64),
            ],
        );
        let reordered: Vec<_> = cs
            .iter()
            .filter(|c| matches!(c.change.kind, ChangeKind::Reordered { .. }))
            .collect();
        assert!(!reordered.is_empty(), "expected reordered changes");
        assert!(reordered.iter().all(|c| c.safety == Safety::Danger));
    }

    #[test]
    fn rename_is_safe_zc() {
        let cs = zc_changes(
            vec![("owner", FieldType::Pubkey), ("amount", FieldType::U64)],
            vec![("owner", FieldType::Pubkey), ("balance", FieldType::U64)],
        );
        let renamed = cs.iter().find(|c| c.change.name == "balance").unwrap();
        assert_eq!(renamed.safety, Safety::Safe);
    }

    #[test]
    fn remove_is_danger_zc() {
        let cs = zc_changes(
            vec![
                ("owner", FieldType::Pubkey),
                ("balance", FieldType::U64),
                ("bump", FieldType::U8),
            ],
            vec![("owner", FieldType::Pubkey), ("bump", FieldType::U8)],
        );
        let removed = cs.iter().find(|c| c.change.name == "balance").unwrap();
        assert_eq!(removed.safety, Safety::Danger);
    }

    #[test]
    fn type_change_same_size_and_align_is_review_zc() {
        // u32 → i32: same size (4), same alignment (4) → Review
        let cs = zc_changes(
            vec![("count", FieldType::U32)],
            vec![("count", FieldType::I32)],
        );
        assert_eq!(cs[0].safety, Safety::Review);
    }

    #[test]
    fn type_change_different_size_is_danger_zc() {
        // u64 → u128: size 8 → 16 → Danger
        let cs = zc_changes(
            vec![("amount", FieldType::U64)],
            vec![("amount", FieldType::U128)],
        );
        assert_eq!(cs[0].safety, Safety::Danger);
    }

    #[test]
    fn type_change_different_align_only_is_danger_zc() {
        // u32 (size=4, align=4) → [u8; 4] (size=4, align=1): same size, different align → Danger
        let ty_old = FieldType::U32;
        let ty_new = FieldType::Array(Box::new(FieldType::U8), 4);
        let cs = zc_changes(vec![("x", ty_old)], vec![("x", ty_new)]);
        assert_eq!(cs[0].safety, Safety::Danger);
    }

    #[test]
    fn type_changed_and_reordered_is_danger_zc() {
        let cs = zc_changes(
            vec![
                ("owner", FieldType::Pubkey),
                ("count", FieldType::U32),
                ("flag", FieldType::Bool),
            ],
            vec![
                ("owner", FieldType::Pubkey),
                ("flag", FieldType::Bool),
                ("count", FieldType::U64), // reordered + type widened
            ],
        );
        let tc = cs.iter().find(|c| c.change.name == "count").unwrap();
        assert_eq!(tc.safety, Safety::Danger);
    }

    // ── Borsh vs ZC contrast ──────────────────────────────────────────────────

    #[test]
    fn borsh_reorder_safe_but_zc_reorder_danger() {
        use crate::borsh::compute_layout;
        use crate::classify::classify_all;
        use crate::idl::FieldDef;

        let old_def = AccountDef {
            name: "Vault".into(),
            fields: vec![
                FieldDef { name: "owner".into(), ty: FieldType::Pubkey, index: 0 },
                FieldDef { name: "balance".into(), ty: FieldType::U64, index: 1 },
                FieldDef { name: "bump".into(), ty: FieldType::U8, index: 2 },
            ],
        };
        let new_def = AccountDef {
            name: "Vault".into(),
            fields: vec![
                FieldDef { name: "owner".into(), ty: FieldType::Pubkey, index: 0 },
                FieldDef { name: "bump".into(), ty: FieldType::U8, index: 1 },
                FieldDef { name: "balance".into(), ty: FieldType::U64, index: 2 },
            ],
        };

        // Borsh: reorder is Safe
        let borsh_changes =
            classify_all(diff(&compute_layout(&old_def), &compute_layout(&new_def)));
        let reordered_borsh: Vec<_> = borsh_changes
            .iter()
            .filter(|c| matches!(c.change.kind, ChangeKind::Reordered { .. }))
            .collect();
        assert!(!reordered_borsh.is_empty());
        assert!(reordered_borsh.iter().all(|c| c.safety == Safety::Safe));

        // ZC: same reorder is Danger
        let old_zc = compute_zc_layout(&old_def).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let zc_changes = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let reordered_zc: Vec<_> = zc_changes
            .iter()
            .filter(|c| matches!(c.change.kind, ChangeKind::Reordered { .. }))
            .collect();
        assert!(!reordered_zc.is_empty());
        assert!(reordered_zc.iter().all(|c| c.safety == Safety::Danger));
    }

    #[test]
    fn borsh_mid_insert_review_but_zc_mid_insert_danger() {
        use crate::borsh::compute_layout;
        use crate::classify::classify_all;
        use crate::idl::FieldDef;

        let old_def = AccountDef {
            name: "Vault".into(),
            fields: vec![
                FieldDef { name: "owner".into(), ty: FieldType::Pubkey, index: 0 },
                FieldDef { name: "bump".into(), ty: FieldType::U8, index: 1 },
            ],
        };
        let new_def = AccountDef {
            name: "Vault".into(),
            fields: vec![
                FieldDef { name: "owner".into(), ty: FieldType::Pubkey, index: 0 },
                FieldDef { name: "balance".into(), ty: FieldType::U64, index: 1 },
                FieldDef { name: "bump".into(), ty: FieldType::U8, index: 2 },
            ],
        };

        // Borsh: mid-insert is Review
        let borsh_cs = classify_all(diff(&compute_layout(&old_def), &compute_layout(&new_def)));
        let added_borsh = borsh_cs.iter().find(|c| c.change.name == "balance").unwrap();
        assert_eq!(added_borsh.safety, Safety::Review);

        // ZC: mid-insert is Danger
        let old_zc = compute_zc_layout(&old_def).unwrap();
        let new_zc = compute_zc_layout(&new_def).unwrap();
        let zc_cs = classify_zc_all(
            diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc)),
            &old_zc,
            &new_zc,
        );
        let added_zc = zc_cs.iter().find(|c| c.change.name == "balance").unwrap();
        assert_eq!(added_zc.safety, Safety::Danger);
    }
}
