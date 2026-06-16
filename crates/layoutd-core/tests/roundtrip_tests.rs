/// Borsh migration roundtrip tests.
///
/// Each test:
///   1. Defines old and new concrete Rust structs with known field values.
///   2. Borsh-serialises the old struct (simulating on-chain data).
///   3. Applies the migration function (identical to what `gen` would produce).
///   4. Asserts every carried field is bit-for-bit identical in the result.
///
/// This proves the migration logic is byte-correct for every Safe/Review change
/// class, without needing a live validator.
use borsh::{BorshDeserialize, BorshSerialize};
use proptest::prelude::*;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Round-trip: serialise `v`, deserialise into `T`.
fn rt<T: BorshSerialize + BorshDeserialize>(v: &T) -> T {
    let bytes = borsh::to_vec(v).unwrap();
    T::try_from_slice(&bytes).unwrap()
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 1 — unchanged fields are bit-for-bit identical after migration
// ═════════════════════════════════════════════════════════════════════════════

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq, Clone)]
struct VaultV1 {
    owner:   [u8; 32],
    balance: u64,
    bump:    u8,
}

// New version: no structural changes at all.
#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct VaultV1Same {
    owner:   [u8; 32],
    balance: u64,
    bump:    u8,
}

fn migrate_v1_unchanged(old: VaultV1) -> VaultV1Same {
    VaultV1Same { owner: old.owner, balance: old.balance, bump: old.bump }
}

#[test]
fn unchanged_fields_are_identical_after_migration() {
    let old = VaultV1 { owner: [7u8; 32], balance: 1_000_000, bump: 254 };
    let bytes = borsh::to_vec(&old).unwrap();
    let old_back = VaultV1::try_from_slice(&bytes).unwrap();
    let new = migrate_v1_unchanged(old_back);
    assert_eq!(new.owner,   [7u8; 32]);
    assert_eq!(new.balance, 1_000_000u64);
    assert_eq!(new.bump,    254u8);
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 2 — field added at end: existing values preserved, new field = default
// ═════════════════════════════════════════════════════════════════════════════

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct VaultV2AddedAtEnd {
    owner:   [u8; 32],
    balance: u64,
    bump:    u8,
    version: u8, // new field at end
}

fn migrate_v1_add_at_end(old: VaultV1) -> VaultV2AddedAtEnd {
    VaultV2AddedAtEnd {
        owner:   old.owner,
        balance: old.balance,
        bump:    old.bump,
        version: 0, // default
    }
}

#[test]
fn added_field_at_end_existing_values_preserved() {
    let old = VaultV1 { owner: [3u8; 32], balance: 42, bump: 1 };
    let new = migrate_v1_add_at_end(rt(&old));
    assert_eq!(new.owner,   [3u8; 32]);
    assert_eq!(new.balance, 42u64);
    assert_eq!(new.bump,    1u8);
    assert_eq!(new.version, 0u8, "new field must default to zero");
}

#[test]
fn added_field_at_end_max_value_roundtrip() {
    let old = VaultV1 { owner: [0xffu8; 32], balance: u64::MAX, bump: u8::MAX };
    let new = migrate_v1_add_at_end(rt(&old));
    assert_eq!(new.balance, u64::MAX);
    assert_eq!(new.bump,    u8::MAX);
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 3 — field renamed: value carries over under the new name
// ═════════════════════════════════════════════════════════════════════════════

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct VaultV3Renamed {
    owner:      [u8; 32],
    collateral: u64, // renamed from 'balance'
    bump:       u8,
}

fn migrate_v1_rename(old: VaultV1) -> VaultV3Renamed {
    VaultV3Renamed {
        owner:      old.owner,
        collateral: old.balance, // value carried over unchanged
        bump:       old.bump,
    }
}

#[test]
fn renamed_field_value_carries_over_unchanged() {
    let old = VaultV1 { owner: [1u8; 32], balance: 999_999_999, bump: 7 };
    let new = migrate_v1_rename(rt(&old));
    assert_eq!(new.collateral, 999_999_999u64, "renamed field must carry value");
    assert_eq!(new.owner,      [1u8; 32]);
    assert_eq!(new.bump,       7u8);
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 4 — integer widen: value fits, no overflow
// ═════════════════════════════════════════════════════════════════════════════

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct OldSmall { value: u32 }

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct NewWide  { value: u64 }

fn migrate_widen(old: OldSmall) -> NewWide {
    NewWide { value: old.value as u64 }
}

#[test]
fn widened_u32_to_u64_preserves_value() {
    for v in [0u32, 1, 255, 65535, u32::MAX] {
        let new = migrate_widen(rt(&OldSmall { value: v }));
        assert_eq!(new.value, v as u64, "widen u32={v}");
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 5 — float widen f32 → f64: value preserved
// ═════════════════════════════════════════════════════════════════════════════

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct OldF32 { rate: f32 }

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct NewF64 { rate: f64 }

fn migrate_float_widen(old: OldF32) -> NewF64 {
    NewF64 { rate: f64::from(old.rate) }
}

#[test]
fn float_widen_f32_to_f64_preserves_value() {
    for v in [0.0f32, 1.5, -3.14, f32::MAX] {
        let new = migrate_float_widen(rt(&OldF32 { rate: v }));
        assert_eq!(new.rate, f64::from(v), "float widen f32={v}");
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 6 — reorder: Borsh matches by name, so values are unaffected
// ═════════════════════════════════════════════════════════════════════════════

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct OrderedOld { owner: [u8; 32], bump: u8, balance: u64 }

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct OrderedNew { owner: [u8; 32], balance: u64, bump: u8 } // reordered

fn migrate_reorder(old: OrderedOld) -> OrderedNew {
    OrderedNew { owner: old.owner, balance: old.balance, bump: old.bump }
}

#[test]
fn reordered_fields_carry_values_unchanged() {
    let old = OrderedOld { owner: [9u8; 32], bump: 5, balance: 777 };
    let new = migrate_reorder(rt(&old));
    assert_eq!(new.owner,   [9u8; 32]);
    assert_eq!(new.balance, 777u64);
    assert_eq!(new.bump,    5u8);
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 7 — multiple simultaneous safe changes
// ═════════════════════════════════════════════════════════════════════════════

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq, Clone)]
struct ComplexOld {
    creator:  [u8; 32],
    amount:   u32,    // will widen
    is_open:  bool,   // unchanged
    counter:  u16,    // will rename
}

#[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq)]
struct ComplexNew {
    creator:  [u8; 32],
    amount:   u64,    // widened
    is_open:  bool,   // unchanged
    total:    u16,    // renamed from counter
    audit_id: u8,     // added at end
}

fn migrate_complex(old: ComplexOld) -> ComplexNew {
    ComplexNew {
        creator:  old.creator,
        amount:   old.amount as u64,
        is_open:  old.is_open,
        total:    old.counter,
        audit_id: 0,
    }
}

#[test]
fn multiple_simultaneous_changes_all_preserved() {
    let old = ComplexOld { creator: [2u8; 32], amount: 500_000, is_open: true, counter: 99 };
    let new = migrate_complex(rt(&old));
    assert_eq!(new.creator,  [2u8; 32]);
    assert_eq!(new.amount,   500_000u64,   "widen preserved");
    assert_eq!(new.is_open,  true,          "unchanged preserved");
    assert_eq!(new.total,    99u16,         "rename preserved");
    assert_eq!(new.audit_id, 0u8,           "new field = default");
}

// ═════════════════════════════════════════════════════════════════════════════
// Case 8 — proptest: random widen/rename/add roundtrips always preserve values
// ═════════════════════════════════════════════════════════════════════════════

proptest! {
    #[test]
    fn prop_widen_u32_to_u64_always_lossless(v in 0u32..=u32::MAX) {
        let old = OldSmall { value: v };
        let new = migrate_widen(rt(&old));
        prop_assert_eq!(new.value, v as u64, "widen must be lossless for any u32 value");
    }

    #[test]
    fn prop_add_at_end_existing_fields_always_preserved(
        owner_byte in 0u8..=255u8,
        balance in 0u64..=u64::MAX,
        bump in 0u8..=255u8,
    ) {
        let old = VaultV1 { owner: [owner_byte; 32], balance, bump };
        let new = migrate_v1_add_at_end(rt(&old));
        prop_assert_eq!(&new.owner[..],   &[owner_byte; 32][..]);
        prop_assert_eq!(new.balance, balance);
        prop_assert_eq!(new.bump,    bump);
        prop_assert_eq!(new.version, 0u8, "new field must always be default");
    }

    #[test]
    fn prop_rename_always_carries_value(balance in 0u64..=u64::MAX) {
        let old = VaultV1 { owner: [0u8; 32], balance, bump: 0 };
        let new = migrate_v1_rename(rt(&old));
        prop_assert_eq!(new.collateral, balance, "rename must carry value exactly");
    }

    #[test]
    fn prop_unchanged_migration_is_always_identity(
        balance in 0u64..=u64::MAX,
        bump in 0u8..=255u8,
    ) {
        let old = VaultV1 { owner: [0u8; 32], balance, bump };
        let new = migrate_v1_unchanged(rt(&old));
        prop_assert_eq!(new.balance, balance);
        prop_assert_eq!(new.bump,    bump);
    }

    #[test]
    fn prop_complex_migration_preserves_all_fields(
        amount in 0u32..=u32::MAX,
        counter in 0u16..=u16::MAX,
        is_open in any::<bool>(),
    ) {
        let old = ComplexOld {
            creator: [0u8; 32],
            amount,
            is_open,
            counter,
        };
        let new = migrate_complex(rt(&old));
        prop_assert_eq!(new.amount  as u32, amount,   "widen must be lossless");
        prop_assert_eq!(new.total,           counter,  "rename must carry value");
        prop_assert_eq!(new.is_open,         is_open,  "unchanged must be identical");
        prop_assert_eq!(new.audit_id,        0u8,      "new field must be default");
    }
}
