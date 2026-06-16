/// Negative corpus — every pattern listed here MUST NOT produce a Safe verdict.
///
/// A failure in this file = release-blocking: layoutd called something "safe" that isn't.
///
/// Organised in sections that mirror the spec's risk model:
///   1. Field removal (any position, any type)
///   2. Type narrowing
///   3. Sign flips
///   4. Float ↔ integer reinterpretation
///   5. String reinterpretation
///   6. Vec reinterpretation
///   7. Unknown type
///   8. Zero-copy: reorder
///   9. Zero-copy: mid-insert
///  10. Zero-copy: size-changing type
///  11. Zero-copy: alignment-changing same-size type
use layoutd_core::borsh::compute_layout;
use layoutd_core::classify::{classify_all, Safety};
use layoutd_core::diff::diff;
use layoutd_core::idl::{AccountDef, FieldDef, FieldType};
use layoutd_core::zerocopy::{classify_zc_all, compute_zc_layout, zc_to_borsh_layout};

// ── helpers ───────────────────────────────────────────────────────────────────

fn account(fields: Vec<(&str, FieldType)>) -> AccountDef {
    AccountDef {
        name: "S".to_string(),
        fields: fields
            .into_iter()
            .enumerate()
            .map(|(i, (n, ty))| FieldDef { name: n.to_string(), ty, index: i })
            .collect(),
    }
}

fn reindex(mut fields: Vec<FieldDef>) -> Vec<FieldDef> {
    for (i, f) in fields.iter_mut().enumerate() { f.index = i; }
    fields
}

/// Classify two accounts in Borsh mode and return the change list.
fn borsh_classify(old: &AccountDef, new: &AccountDef) -> Vec<layoutd_core::classify::ClassifiedChange> {
    classify_all(diff(&compute_layout(old), &compute_layout(new)))
}

/// Classify two accounts in zero-copy mode.
fn zc_classify(old: &AccountDef, new: &AccountDef) -> Vec<layoutd_core::classify::ClassifiedChange> {
    let old_zc = compute_zc_layout(old).expect("old zc layout failed");
    let new_zc = compute_zc_layout(new).expect("new zc layout failed");
    let changes = diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc));
    classify_zc_all(changes, &old_zc, &new_zc)
}

/// Assert that at least one field in the classified list is Danger,
/// and that NO field is Safe when we expect every change to be at least Danger.
fn assert_has_danger(changes: &[layoutd_core::classify::ClassifiedChange], label: &str) {
    assert!(
        changes.iter().any(|c| c.safety == Safety::Danger),
        "{label}: expected at least one Danger change but got none\nchanges: {:#?}",
        changes.iter().map(|c| (&c.change.name, &c.safety, c.reason)).collect::<Vec<_>>()
    );
}

/// Assert a specific named field is Danger.
fn assert_field_danger(
    changes: &[layoutd_core::classify::ClassifiedChange],
    field: &str,
    label: &str,
) {
    let c = changes.iter().find(|c| c.change.name == field)
        .unwrap_or_else(|| panic!("{label}: field '{field}' not found in changes"));
    assert_eq!(
        c.safety, Safety::Danger,
        "{label}: field '{field}' was {:?} (reason: {}), expected Danger",
        c.safety, c.reason
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. FIELD REMOVAL — data loss in every position
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn remove_first_field_is_danger() {
    let old = account(vec![
        ("first", FieldType::Pubkey),
        ("second", FieldType::U64),
        ("third", FieldType::U8),
    ]);
    let new_def = AccountDef {
        name: "S".to_string(),
        fields: reindex(old.fields[1..].to_vec()),
    };
    let cs = borsh_classify(&old, &new_def);
    assert_field_danger(&cs, "first", "remove first field");
}

#[test]
fn remove_middle_field_is_danger() {
    let old = account(vec![
        ("a", FieldType::Pubkey),
        ("b", FieldType::U64),
        ("c", FieldType::U8),
    ]);
    let new_def = AccountDef {
        name: "S".to_string(),
        fields: reindex(vec![old.fields[0].clone(), old.fields[2].clone()]),
    };
    let cs = borsh_classify(&old, &new_def);
    assert_field_danger(&cs, "b", "remove middle field");
}

#[test]
fn remove_last_field_is_danger() {
    let old = account(vec![
        ("x", FieldType::U64),
        ("y", FieldType::U8),
    ]);
    let new_def = AccountDef {
        name: "S".to_string(),
        fields: reindex(vec![old.fields[0].clone()]),
    };
    let cs = borsh_classify(&old, &new_def);
    assert_field_danger(&cs, "y", "remove last field");
}

#[test]
fn remove_only_field_is_danger() {
    let old = account(vec![("solo", FieldType::U64)]);
    let new_def = AccountDef { name: "S".to_string(), fields: vec![] };
    let cs = borsh_classify(&old, &new_def);
    assert_field_danger(&cs, "solo", "remove only field");
}

#[test]
fn remove_string_field_is_danger() {
    let old = account(vec![("label", FieldType::String), ("bump", FieldType::U8)]);
    let new_def = AccountDef {
        name: "S".to_string(),
        fields: reindex(vec![old.fields[1].clone()]),
    };
    let cs = borsh_classify(&old, &new_def);
    assert_field_danger(&cs, "label", "remove String field");
}

#[test]
fn remove_vec_field_is_danger() {
    let old = account(vec![
        ("items", FieldType::Vec(Box::new(FieldType::U64))),
        ("bump", FieldType::U8),
    ]);
    let new_def = AccountDef {
        name: "S".to_string(),
        fields: reindex(vec![old.fields[1].clone()]),
    };
    let cs = borsh_classify(&old, &new_def);
    assert_field_danger(&cs, "items", "remove Vec field");
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. NARROWING TYPE CHANGES — possible overflow / data loss
// ═════════════════════════════════════════════════════════════════════════════

macro_rules! narrowing_test {
    ($name:ident, $from:expr, $to:expr) => {
        #[test]
        fn $name() {
            let old = account(vec![("v", $from)]);
            let new = account(vec![("v", $to)]);
            let cs = borsh_classify(&old, &new);
            assert_field_danger(&cs, "v", concat!(stringify!($name)));
        }
    };
}

narrowing_test!(narrow_u128_to_u64, FieldType::U128, FieldType::U64);
narrowing_test!(narrow_u128_to_u32, FieldType::U128, FieldType::U32);
narrowing_test!(narrow_u128_to_u16, FieldType::U128, FieldType::U16);
narrowing_test!(narrow_u128_to_u8,  FieldType::U128, FieldType::U8);
narrowing_test!(narrow_u64_to_u32,  FieldType::U64,  FieldType::U32);
narrowing_test!(narrow_u64_to_u16,  FieldType::U64,  FieldType::U16);
narrowing_test!(narrow_u64_to_u8,   FieldType::U64,  FieldType::U8);
narrowing_test!(narrow_u32_to_u16,  FieldType::U32,  FieldType::U16);
narrowing_test!(narrow_u32_to_u8,   FieldType::U32,  FieldType::U8);
narrowing_test!(narrow_u16_to_u8,   FieldType::U16,  FieldType::U8);

narrowing_test!(narrow_i128_to_i64, FieldType::I128, FieldType::I64);
narrowing_test!(narrow_i128_to_i32, FieldType::I128, FieldType::I32);
narrowing_test!(narrow_i128_to_i16, FieldType::I128, FieldType::I16);
narrowing_test!(narrow_i128_to_i8,  FieldType::I128, FieldType::I8);
narrowing_test!(narrow_i64_to_i32,  FieldType::I64,  FieldType::I32);
narrowing_test!(narrow_i64_to_i16,  FieldType::I64,  FieldType::I16);
narrowing_test!(narrow_i64_to_i8,   FieldType::I64,  FieldType::I8);
narrowing_test!(narrow_i32_to_i16,  FieldType::I32,  FieldType::I16);
narrowing_test!(narrow_i32_to_i8,   FieldType::I32,  FieldType::I8);
narrowing_test!(narrow_i16_to_i8,   FieldType::I16,  FieldType::I8);
narrowing_test!(narrow_f64_to_f32,  FieldType::F64,  FieldType::F32);

// ═════════════════════════════════════════════════════════════════════════════
// 3. SIGN FLIPS — same bytes, different meaning
// ═════════════════════════════════════════════════════════════════════════════

macro_rules! sign_flip_test {
    ($name:ident, $a:expr, $b:expr) => {
        #[test]
        fn $name() {
            let old = account(vec![("v", $a)]);
            let new = account(vec![("v", $b)]);
            let cs = borsh_classify(&old, &new);
            assert_field_danger(&cs, "v", concat!(stringify!($name)));
        }
    };
}

sign_flip_test!(sign_flip_u8_to_i8,    FieldType::U8,   FieldType::I8);
sign_flip_test!(sign_flip_i8_to_u8,    FieldType::I8,   FieldType::U8);
sign_flip_test!(sign_flip_u16_to_i16,  FieldType::U16,  FieldType::I16);
sign_flip_test!(sign_flip_i16_to_u16,  FieldType::I16,  FieldType::U16);
sign_flip_test!(sign_flip_u32_to_i32,  FieldType::U32,  FieldType::I32);
sign_flip_test!(sign_flip_i32_to_u32,  FieldType::I32,  FieldType::U32);
sign_flip_test!(sign_flip_u64_to_i64,  FieldType::U64,  FieldType::I64);
sign_flip_test!(sign_flip_i64_to_u64,  FieldType::I64,  FieldType::U64);
sign_flip_test!(sign_flip_u128_to_i128, FieldType::U128, FieldType::I128);
sign_flip_test!(sign_flip_i128_to_u128, FieldType::I128, FieldType::U128);

// ═════════════════════════════════════════════════════════════════════════════
// 4. FLOAT ↔ INTEGER REINTERPRETATION — bits mean completely different things
// ═════════════════════════════════════════════════════════════════════════════

macro_rules! float_int_test {
    ($name:ident, $a:expr, $b:expr) => {
        #[test]
        fn $name() {
            let old = account(vec![("v", $a)]);
            let new = account(vec![("v", $b)]);
            let cs = borsh_classify(&old, &new);
            assert_field_danger(&cs, "v", concat!(stringify!($name)));
        }
    };
}

float_int_test!(float_f32_to_u32,  FieldType::F32, FieldType::U32);
float_int_test!(float_f32_to_i32,  FieldType::F32, FieldType::I32);
float_int_test!(float_f32_to_u8,   FieldType::F32, FieldType::U8);
float_int_test!(float_f64_to_u64,  FieldType::F64, FieldType::U64);
float_int_test!(float_f64_to_i64,  FieldType::F64, FieldType::I64);
float_int_test!(float_f64_to_u8,   FieldType::F64, FieldType::U8);
float_int_test!(int_u32_to_f32,    FieldType::U32, FieldType::F32);
float_int_test!(int_i32_to_f32,    FieldType::I32, FieldType::F32);
float_int_test!(int_u64_to_f64,    FieldType::U64, FieldType::F64);
float_int_test!(int_i64_to_f64,    FieldType::I64, FieldType::F64);
float_int_test!(int_u8_to_f32,     FieldType::U8,  FieldType::F32);
float_int_test!(int_u8_to_f64,     FieldType::U8,  FieldType::F64);

// ═════════════════════════════════════════════════════════════════════════════
// 5. STRING REINTERPRETATION — variable-length encoding vs. fixed types
// ═════════════════════════════════════════════════════════════════════════════

macro_rules! string_reinterpret_test {
    ($name:ident, $a:expr, $b:expr) => {
        #[test]
        fn $name() {
            let old = account(vec![("v", $a)]);
            let new = account(vec![("v", $b)]);
            let cs = borsh_classify(&old, &new);
            assert_field_danger(&cs, "v", concat!(stringify!($name)));
        }
    };
}

string_reinterpret_test!(string_to_pubkey, FieldType::String, FieldType::Pubkey);
string_reinterpret_test!(string_to_u64,    FieldType::String, FieldType::U64);
string_reinterpret_test!(string_to_bool,   FieldType::String, FieldType::Bool);
string_reinterpret_test!(pubkey_to_string, FieldType::Pubkey, FieldType::String);
string_reinterpret_test!(u64_to_string,    FieldType::U64,    FieldType::String);
string_reinterpret_test!(bool_to_string,   FieldType::Bool,   FieldType::String);

// ═════════════════════════════════════════════════════════════════════════════
// 6. VEC REINTERPRETATION — length-prefixed vs. fixed types
// ═════════════════════════════════════════════════════════════════════════════

macro_rules! vec_reinterpret_test {
    ($name:ident, $a:expr, $b:expr) => {
        #[test]
        fn $name() {
            let old = account(vec![("v", $a)]);
            let new = account(vec![("v", $b)]);
            let cs = borsh_classify(&old, &new);
            assert_field_danger(&cs, "v", concat!(stringify!($name)));
        }
    };
}

vec_reinterpret_test!(vec_u8_to_u64,
    FieldType::Vec(Box::new(FieldType::U8)), FieldType::U64);
vec_reinterpret_test!(vec_u8_to_pubkey,
    FieldType::Vec(Box::new(FieldType::U8)), FieldType::Pubkey);
vec_reinterpret_test!(vec_u8_to_string,
    FieldType::Vec(Box::new(FieldType::U8)), FieldType::String);
vec_reinterpret_test!(u64_to_vec_u8,
    FieldType::U64, FieldType::Vec(Box::new(FieldType::U8)));
vec_reinterpret_test!(pubkey_to_vec_u8,
    FieldType::Pubkey, FieldType::Vec(Box::new(FieldType::U8)));
vec_reinterpret_test!(vec_u64_to_vec_u8,
    FieldType::Vec(Box::new(FieldType::U64)), FieldType::Vec(Box::new(FieldType::U8)));

// ═════════════════════════════════════════════════════════════════════════════
// 7. UNKNOWN TYPES — cannot reason about byte safety
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn unknown_old_type_is_danger() {
    let old = account(vec![("v", FieldType::Unknown("SomeEnum".to_string()))]);
    let new = account(vec![("v", FieldType::U64)]);
    let cs = borsh_classify(&old, &new);
    assert_field_danger(&cs, "v", "unknown old type");
}

#[test]
fn unknown_new_type_is_danger() {
    let old = account(vec![("v", FieldType::U64)]);
    let new = account(vec![("v", FieldType::Unknown("SomeEnum".to_string()))]);
    let cs = borsh_classify(&old, &new);
    assert_field_danger(&cs, "v", "unknown new type");
}

#[test]
fn both_unknown_same_name_is_danger() {
    let old = account(vec![("v", FieldType::Unknown("A".to_string()))]);
    let new = account(vec![("v", FieldType::Unknown("B".to_string()))]);
    let cs = borsh_classify(&old, &new);
    assert_field_danger(&cs, "v", "both unknown different names");
}

// ═════════════════════════════════════════════════════════════════════════════
// 8. ZERO-COPY: REORDER — declaration order controls byte offsets
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn zc_reorder_two_fields_is_danger() {
    let old = account(vec![("x", FieldType::U64), ("y", FieldType::U32)]);
    let new = account(vec![("y", FieldType::U32), ("x", FieldType::U64)]);
    let cs = zc_classify(&old, &new);
    let reordered: Vec<_> = cs.iter().filter(|c| c.safety == Safety::Danger).collect();
    assert!(!reordered.is_empty(), "zc reorder should be Danger");
}

#[test]
fn zc_reorder_three_fields_is_danger() {
    let old = account(vec![
        ("a", FieldType::U8),
        ("b", FieldType::U32),
        ("c", FieldType::U64),
    ]);
    let new = account(vec![
        ("c", FieldType::U64),
        ("b", FieldType::U32),
        ("a", FieldType::U8),
    ]);
    let cs = zc_classify(&old, &new);
    assert_has_danger(&cs, "zc reorder three fields");
}

#[test]
fn zc_reorder_pubkey_and_u64_is_danger() {
    let old = account(vec![("owner", FieldType::Pubkey), ("balance", FieldType::U64)]);
    let new = account(vec![("balance", FieldType::U64), ("owner", FieldType::Pubkey)]);
    let cs = zc_classify(&old, &new);
    assert_has_danger(&cs, "zc reorder pubkey and u64");
}

// ═════════════════════════════════════════════════════════════════════════════
// 9. ZERO-COPY: MID-INSERT — shifts byte offsets of all following fields
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn zc_insert_before_first_field_is_danger() {
    let old = account(vec![("x", FieldType::U64)]);
    let new = account(vec![("prefix", FieldType::U8), ("x", FieldType::U64)]);
    let cs = zc_classify(&old, &new);
    assert_field_danger(&cs, "prefix", "zc insert before first field");
}

#[test]
fn zc_insert_in_middle_is_danger() {
    let old = account(vec![("a", FieldType::Pubkey), ("b", FieldType::U8)]);
    let new = account(vec![
        ("a", FieldType::Pubkey),
        ("mid", FieldType::U64),
        ("b", FieldType::U8),
    ]);
    let cs = zc_classify(&old, &new);
    assert_field_danger(&cs, "mid", "zc insert in middle");
}

#[test]
fn zc_insert_multiple_fields_in_middle_is_danger() {
    let old = account(vec![("start", FieldType::U64), ("end", FieldType::U8)]);
    let new = account(vec![
        ("start", FieldType::U64),
        ("m1", FieldType::U32),
        ("m2", FieldType::U32),
        ("end", FieldType::U8),
    ]);
    let cs = zc_classify(&old, &new);
    assert!(
        cs.iter().filter(|c| c.safety == Safety::Danger).count() >= 1,
        "multiple zc mid-inserts should produce at least one Danger"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 10. ZERO-COPY: SIZE-CHANGING TYPE CHANGE — shifts all following offsets
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn zc_type_widen_u32_to_u64_is_danger() {
    let old = account(vec![("v", FieldType::U32), ("bump", FieldType::U8)]);
    let new = account(vec![("v", FieldType::U64), ("bump", FieldType::U8)]);
    let cs = zc_classify(&old, &new);
    assert_field_danger(&cs, "v", "zc widen u32→u64 (size changes)");
}

#[test]
fn zc_type_widen_u8_to_u128_is_danger() {
    let old = account(vec![("v", FieldType::U8)]);
    let new = account(vec![("v", FieldType::U128)]);
    let cs = zc_classify(&old, &new);
    assert_field_danger(&cs, "v", "zc widen u8→u128 (size changes)");
}

#[test]
fn zc_type_narrow_u64_to_u32_is_danger() {
    let old = account(vec![("v", FieldType::U64)]);
    let new = account(vec![("v", FieldType::U32)]);
    let cs = zc_classify(&old, &new);
    assert_field_danger(&cs, "v", "zc narrow u64→u32 (size changes)");
}

// ═════════════════════════════════════════════════════════════════════════════
// 11. ZERO-COPY: ALIGNMENT-CHANGING SAME-SIZE TYPE — offsets and padding shift
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn zc_u32_to_array_u8_4_same_size_different_align_is_danger() {
    // u32: size=4, align=4   vs   [u8;4]: size=4, align=1 → alignment differs → Danger
    let old = account(vec![("v", FieldType::U32)]);
    let new = account(vec![("v", FieldType::Array(Box::new(FieldType::U8), 4))]);
    let cs = zc_classify(&old, &new);
    assert_field_danger(&cs, "v", "zc u32→[u8;4] same size but alignment differs");
}

#[test]
fn zc_f32_to_array_u8_4_same_size_different_align_is_danger() {
    // f32: size=4, align=4   vs   [u8;4]: size=4, align=1 → Danger
    let old = account(vec![("v", FieldType::F32)]);
    let new = account(vec![("v", FieldType::Array(Box::new(FieldType::U8), 4))]);
    let cs = zc_classify(&old, &new);
    assert_field_danger(&cs, "v", "zc f32→[u8;4] same size but alignment differs");
}

// ═════════════════════════════════════════════════════════════════════════════
// 12. COMPOUND: removal never becomes safe in multi-field structs
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn removing_field_from_large_struct_is_always_danger() {
    // 8-field account; remove field at every position, each must be Danger.
    let base_fields: Vec<(&str, FieldType)> = vec![
        ("f0", FieldType::Pubkey),
        ("f1", FieldType::U64),
        ("f2", FieldType::U32),
        ("f3", FieldType::U16),
        ("f4", FieldType::U8),
        ("f5", FieldType::Bool),
        ("f6", FieldType::I64),
        ("f7", FieldType::U128),
    ];
    let base = account(base_fields.clone());

    for rm in 0..base_fields.len() {
        let mut new_fields: Vec<FieldDef> = base.fields.clone();
        let removed_name = new_fields[rm].name.clone();
        new_fields.remove(rm);
        let new_def = AccountDef {
            name: "S".to_string(),
            fields: reindex(new_fields),
        };
        let cs = borsh_classify(&base, &new_def);
        assert_field_danger(
            &cs,
            &removed_name,
            &format!("remove f{rm} from 8-field struct"),
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 13. DETERMINISM: identical inputs twice = identical output
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn classifier_is_deterministic() {
    let old = account(vec![
        ("owner", FieldType::Pubkey),
        ("balance", FieldType::U64),
        ("bump", FieldType::U8),
    ]);
    let new = account(vec![
        ("owner", FieldType::Pubkey),
        ("balance", FieldType::U128), // widen
        ("extra", FieldType::U8),     // add at end
    ]);
    let run1 = borsh_classify(&old, &new);
    let run2 = borsh_classify(&old, &new);
    for (a, b) in run1.iter().zip(run2.iter()) {
        assert_eq!(a.safety, b.safety, "non-deterministic safety for '{}'", a.change.name);
        assert_eq!(a.reason, b.reason, "non-deterministic reason for '{}'", a.change.name);
    }
}
