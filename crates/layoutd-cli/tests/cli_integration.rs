/// CLI integration tests.
///
/// Each test spawns the real `layoutd` binary and asserts on:
///   - exit code
///   - stdout content
///   - produced files (SARIF, etc.)
///
/// Fixtures live in tests/fixtures/ so they are committed to the repo.
use std::io::Write as _;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::NamedTempFile;

// ── path helpers ──────────────────────────────────────────────────────────────

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn layoutd() -> Command {
    Command::cargo_bin("layoutd").unwrap()
}

fn write_temp(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{content}").unwrap();
    f
}

// ══════════════════════════════════════════════════════════════════════════════
// `diff` command
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn diff_identical_idls_exits_0() {
    layoutd()
        .args(["diff", fixture("vault_v1.json").to_str().unwrap(),
                       fixture("vault_v1.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success();
}

#[test]
fn diff_prints_field_names_and_safety_tags() {
    let out = layoutd()
        .args(["diff", fixture("vault_v1.json").to_str().unwrap(),
                       fixture("vault_v2_safe.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("version"), "should mention the new field");
    // Existing unchanged fields (owner, balance, bump) are SAFE.
    assert!(stdout.contains("SAFE"),   "should print SAFE for unchanged fields");
    // The appended field (version) is DANGER: old accounts have no bytes for it.
    assert!(stdout.contains("DANGER"), "should print DANGER for appended field to exact-sized account");
}

#[test]
fn diff_danger_prints_danger_tag_but_still_exits_0() {
    // `diff` is informational only — it always exits 0.
    let out = layoutd()
        .args(["diff", fixture("vault_v1.json").to_str().unwrap(),
                       fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("DANGER"), "diff should print DANGER for removal");
}

#[test]
fn diff_shows_review_for_widen() {
    let out = layoutd()
        .args(["diff", fixture("vault_v1.json").to_str().unwrap(),
                       fixture("vault_v2_widen.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("DANGER"), "widen should produce DANGER — byte layout expands");
}

#[test]
fn diff_with_source_rs_file_exits_0() {
    layoutd()
        .args(["diff", fixture("vault_zc.rs").to_str().unwrap(),
                       fixture("vault_zc.rs").to_str().unwrap(),
               "--account", "VaultZC", "--zero-copy"])
        .assert()
        .success();
}

// ══════════════════════════════════════════════════════════════════════════════
// `gen` command
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn gen_always_exits_0() {
    // gen is code-generation only — it always exits 0 regardless of safety level.
    layoutd()
        .args(["gen", fixture("vault_v1.json").to_str().unwrap(),
                      fixture("vault_v2_safe.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success();
}

#[test]
fn gen_outputs_rust_migration_struct() {
    let out = layoutd()
        .args(["gen", fixture("vault_v1.json").to_str().unwrap(),
                      fixture("vault_v2_safe.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("impl Migration"), "gen must emit impl Migration block");
    assert!(stdout.contains("pub fn migrate"), "gen must emit migrate function");
    assert!(stdout.contains("Vault"),          "gen must reference account name");
}

#[test]
fn gen_includes_size_comment_when_field_added() {
    let out = layoutd()
        .args(["gen", fixture("vault_v1.json").to_str().unwrap(),
                      fixture("vault_v2_safe.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    // A field was added → size grew → size comment must be present.
    assert!(stdout.contains("Size:"), "gen must emit size comment when account grows");
    assert!(stdout.contains("realloc"), "gen must mention realloc in size comment");
}

#[test]
fn gen_emits_danger_warning_for_removed_field() {
    let out = layoutd()
        .args(["gen", fixture("vault_v1.json").to_str().unwrap(),
                      fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("DANGER"),   "gen must mark removed field as DANGER");
    assert!(stdout.contains("WARNING"),  "gen must print WARNING header for danger");
}

// ══════════════════════════════════════════════════════════════════════════════
// `check` command — exit codes
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn check_appended_field_to_fixed_account_exits_1() {
    // vault_v2_safe.json adds version:u8 at the end of a fixed-size Vault.
    // Old accounts have no bytes for it — Borsh hits end-of-buffer. Must exit 1.
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_safe.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn check_identical_idls_exits_0() {
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v1.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success();
}

#[test]
fn check_danger_exits_1() {
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn check_widen_exits_1() {
    // A widen is now DANGER (byte size changes) — CI must fail.
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_widen.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn check_danger_stdout_names_field_and_reason() {
    let out = layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("DANGER"),  "check must print DANGER for field removal");
    assert!(stdout.contains("balance"), "check must name the removed field");
}

#[test]
fn check_prints_ok_when_all_safe() {
    let out = layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v1.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("OK"), "check must print OK when all safe");
}

#[test]
fn check_suggests_gen_on_failure() {
    let out = layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(stdout.contains("gen"), "check failure should suggest running `layoutd gen`");
}

// ══════════════════════════════════════════════════════════════════════════════
// `check --sarif` — SARIF 2.1.0 output
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn check_sarif_creates_valid_json_file() {
    let sarif_file = tempfile::NamedTempFile::new().unwrap();
    let sarif_path = sarif_file.path().to_str().unwrap().to_string();

    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault",
               "--sarif", &sarif_path])
        .assert()
        .failure(); // danger present → exit 1

    let content = std::fs::read_to_string(&sarif_path).expect("SARIF file must be created");
    let doc: serde_json::Value = serde_json::from_str(&content)
        .expect("SARIF output must be valid JSON");

    assert_eq!(doc["version"], "2.1.0", "SARIF version must be 2.1.0");
    assert!(doc["runs"].is_array(), "SARIF must have a 'runs' array");
    let runs = doc["runs"].as_array().unwrap();
    assert!(!runs.is_empty(), "SARIF runs must not be empty");
    assert!(runs[0]["results"].is_array(), "SARIF run must have a 'results' array");
}

#[test]
fn check_sarif_results_contain_rule_ids() {
    let sarif_file = tempfile::NamedTempFile::new().unwrap();
    let sarif_path = sarif_file.path().to_str().unwrap().to_string();

    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault",
               "--sarif", &sarif_path])
        .assert()
        .failure();

    let content = std::fs::read_to_string(&sarif_path).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&content).unwrap();
    let results = doc["runs"][0]["results"].as_array().unwrap();
    assert!(!results.is_empty(), "SARIF must have at least one result for a danger");
    for result in results {
        let rule_id = result["ruleId"].as_str().unwrap_or("");
        assert!(
            rule_id.starts_with("LD"),
            "every SARIF result must have an LD-prefixed rule ID, got '{rule_id}'"
        );
    }
}

#[test]
fn check_sarif_written_even_when_safe() {
    let sarif_file = tempfile::NamedTempFile::new().unwrap();
    let sarif_path = sarif_file.path().to_str().unwrap().to_string();

    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v1.json").to_str().unwrap(),
               "--account", "Vault",
               "--sarif", &sarif_path])
        .assert()
        .success();

    // File must still exist and be valid JSON (empty results array).
    let content = std::fs::read_to_string(&sarif_path).expect("SARIF file must exist even for safe runs");
    let doc: serde_json::Value = serde_json::from_str(&content).expect("must be valid JSON");
    assert_eq!(doc["version"], "2.1.0");
    let results = doc["runs"][0]["results"].as_array().unwrap();
    assert!(results.is_empty(), "safe run must produce empty SARIF results");
}

// ══════════════════════════════════════════════════════════════════════════════
// `check --ack` — acknowledgement file
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn check_ack_named_danger_exits_0() {
    // vault_v2_danger removes 'balance', which shifts 'bump' from index 2→1 (reordered).
    // Both changes are DANGER and must be acknowledged.
    let ack = write_temp(r#"{
        "acknowledged": [
            { "account": "Vault", "field": "balance", "change": "removed",   "note": "intentional" },
            { "account": "Vault", "field": "bump",    "change": "reordered", "note": "side-effect of balance removal" }
        ]
    }"#);
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault",
               "--ack", ack.path().to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn check_ack_wrong_field_name_still_fails() {
    // Ack names 'other_field' but danger is 'balance' → must still fail.
    let ack = write_temp(r#"{
        "acknowledged": [
            { "account": "Vault", "field": "other_field", "change": "removed", "note": "typo" }
        ]
    }"#);
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v2_danger.json").to_str().unwrap(),
               "--account", "Vault",
               "--ack", ack.path().to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn check_ack_stale_entry_is_reported() {
    // Ack names a field that no longer has any danger → stale ack warning.
    let ack = write_temp(r#"{
        "acknowledged": [
            { "account": "Vault", "field": "non_existent_field", "change": "removed", "note": "stale" }
        ]
    }"#);
    let out = layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v1.json").to_str().unwrap(), // identical → no dangers
               "--account", "Vault",
               "--ack", ack.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(
        stdout.contains("WARN") || stdout.contains("stale"),
        "stale ack must be reported: {stdout}"
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// `check --hints` — rename disambiguation
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn check_with_hints_file_exits_0_for_hinted_rename() {
    // Cross-position rename: 'balance' renamed to 'collateral' AND moved to a
    // different index (swapped with bump). Auto-detection requires same index,
    // so without a hint it sees balance=Removed (Danger) + collateral=Added.
    //
    // vault_v1:   [owner(pubkey,0), balance(u64,1), bump(u8,2)]
    // v2_swapped: [owner(pubkey,0), bump(u8,1),     collateral(u64,2)]
    let v2_swapped = write_temp(r#"{
        "types": [{
            "name": "Vault",
            "type": {
                "kind": "struct",
                "fields": [
                    { "name": "owner",      "type": "pubkey" },
                    { "name": "bump",       "type": "u8"     },
                    { "name": "collateral", "type": "u64"    }
                ]
            }
        }]
    }"#);

    // Without hint: balance at idx 1, collateral at idx 2 — different indices,
    // auto-detection won't pair them → balance=Removed (Danger) → exit 1.
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        v2_swapped.path().to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .failure()
        .code(1);

    // With hint: balance→collateral is forced regardless of index change → Renamed (Safe).
    // bump moved from index 2→1 (reordered = Danger), so also ack that.
    let hints = write_temp(r#"{
        "renames": [{ "account": "Vault", "from": "balance", "to": "collateral" }]
    }"#);
    let ack = write_temp(r#"{
        "acknowledged": [
            { "account": "Vault", "field": "bump", "change": "reordered", "note": "side-effect of swap" }
        ]
    }"#);
    layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        v2_swapped.path().to_str().unwrap(),
               "--account", "Vault",
               "--hints", hints.path().to_str().unwrap(),
               "--ack",   ack.path().to_str().unwrap()])
        .assert()
        .success();
}

// ══════════════════════════════════════════════════════════════════════════════
// `check --zero-copy` — stricter rules
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn check_zero_copy_reorder_exits_1() {
    // Identical struct, but in ZC mode reorder is Danger.
    let old_rs = write_temp(r#"
        #[account(zero_copy)] pub struct VaultZC {
            pub owner: Pubkey, pub balance: u64, pub bump: u8,
        }
    "#);
    let new_rs = write_temp(r#"
        #[account(zero_copy)] pub struct VaultZC {
            pub owner: Pubkey, pub bump: u8, pub balance: u64,
        }
    "#);
    // Rename temp files to .rs so the CLI picks the source adapter.
    let old_path = {
        let p = tempfile::Builder::new().suffix(".rs").tempfile().unwrap();
        std::fs::write(p.path(), std::fs::read_to_string(old_rs.path()).unwrap()).unwrap();
        p
    };
    let new_path = {
        let p = tempfile::Builder::new().suffix(".rs").tempfile().unwrap();
        std::fs::write(p.path(), std::fs::read_to_string(new_rs.path()).unwrap()).unwrap();
        p
    };
    layoutd()
        .args(["check", old_path.path().to_str().unwrap(),
                        new_path.path().to_str().unwrap(),
               "--account", "VaultZC", "--zero-copy"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn check_zero_copy_add_at_end_exits_0() {
    let old_rs = tempfile::Builder::new().suffix(".rs").tempfile().unwrap();
    std::fs::write(old_rs.path(), r#"
        #[account(zero_copy)] pub struct VaultZC {
            pub owner: Pubkey, pub balance: u64,
        }
    "#).unwrap();
    let new_rs = tempfile::Builder::new().suffix(".rs").tempfile().unwrap();
    std::fs::write(new_rs.path(), r#"
        #[account(zero_copy)] pub struct VaultZC {
            pub owner: Pubkey, pub balance: u64, pub bump: u8,
        }
    "#).unwrap();
    layoutd()
        .args(["check", old_rs.path().to_str().unwrap(),
                        new_rs.path().to_str().unwrap(),
               "--account", "VaultZC", "--zero-copy"])
        .assert()
        .success();
}

// ══════════════════════════════════════════════════════════════════════════════
// Determinism
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn check_is_deterministic_across_two_runs() {
    fn run_check() -> (Vec<u8>, i32) {
        let sarif_file = tempfile::NamedTempFile::new().unwrap();
        let result = layoutd()
            .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                            fixture("vault_v2_danger.json").to_str().unwrap(),
                   "--account", "Vault",
                   "--sarif", sarif_file.path().to_str().unwrap()])
            .output()
            .unwrap();
        let content = std::fs::read_to_string(sarif_file.path()).unwrap_or_default();
        let code = result.status.code().unwrap_or(-1);
        (content.into_bytes(), code)
    }
    let (out1, code1) = run_check();
    let (out2, code2) = run_check();
    assert_eq!(code1, code2, "exit codes must be identical across runs");
    assert_eq!(out1, out2, "SARIF output must be identical across runs");
}

// ══════════════════════════════════════════════════════════════════════════════
// Robustness
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn missing_idl_file_exits_nonzero_with_message() {
    let out = layoutd()
        .args(["check", "/nonexistent/old.json", "/nonexistent/new.json",
               "--account", "Vault"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).unwrap();
    assert!(!stderr.is_empty(), "missing file must produce stderr message");
}

#[test]
fn unknown_account_in_idl_exits_nonzero_with_message() {
    let out = layoutd()
        .args(["check", fixture("vault_v1.json").to_str().unwrap(),
                        fixture("vault_v1.json").to_str().unwrap(),
               "--account", "DoesNotExist"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).unwrap();
    assert!(stderr.contains("DoesNotExist"), "error must name the missing account");
}

#[test]
fn malformed_json_idl_exits_nonzero() {
    let bad = write_temp("this is not json !!!!");
    layoutd()
        .args(["check", bad.path().to_str().unwrap(),
                        bad.path().to_str().unwrap(),
               "--account", "Vault"])
        .assert()
        .failure();
}
