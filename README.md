# layoutd

Off-chain CLI for the Solana/Anchor ecosystem that diffs account struct layouts and classifies every field change as **Safe**, **Review**, or **Danger** before you ship a migration.

```
layoutd diff old.json new.json --account Vault
layoutd check old.json new.json --account Vault   # exits 1 on Danger → blocks CI
layoutd gen   old.json new.json --account Vault   # emits a Rust migration scaffold
```

## Install

```bash
cargo install layoutd
```

Or install the latest from GitHub without publishing:

```bash
cargo install --git https://github.com/Rudraprajapati2612/layoutd-cli --bin layoutd
```

## What it does

Anchor encodes accounts with Borsh (sequential field encoding). If you change a struct definition in a way that shifts byte offsets for existing data, every account already on-chain becomes unreadable. `layoutd` catches those mistakes in CI before they hit mainnet.

### Safety classification

| Change | Borsh | Zero-copy (`repr(C)`) |
|---|---|---|
| Field added at end | Safe | Safe |
| Field renamed (same type, same position) | Safe | Safe |
| Field type widened (u32 → u64) | Review | Danger |
| Field removed | Danger | Danger |
| Field reordered | Safe* | Danger |
| Field inserted in the middle | Danger | Danger |

\* Borsh is position-independent for deserialization; reorder only matters for on-chain size.

## Usage

### `diff` — inspect changes

```bash
layoutd diff old_idl.json new_idl.json --account EscrowVault
```

Accepts Anchor IDL JSON **or** a Rust source file (`.rs`):

```bash
layoutd diff vault_v1.rs vault_v2.rs --account Vault --zero-copy
```

### `check` — CI gate

```bash
layoutd check old.json new.json --account Vault
# exits 0 → all changes safe
# exits 1 → at least one Danger (blocks CI)
```

Output a SARIF file for GitHub PR annotations:

```bash
layoutd check old.json new.json --account Vault --sarif results.sarif
```

### `gen` — migration scaffold

```bash
layoutd gen old.json new.json --account Vault
```

Emits a Rust `impl Migration` block with a `migrate()` function, pre-filled for every detected change, plus a `// Size:` comment telling you whether you need `realloc()`.

### Flags

| Flag | Description |
|---|---|
| `--zero-copy` | Analyse as a `repr(C)` zero-copy account (stricter rules) |
| `--ack <file>` | JSON file of explicitly acknowledged Danger changes (lets them pass CI) |
| `--hints <file>` | JSON file of explicit rename pairs (for cross-position renames auto-detection misses) |
| `--sarif <file>` | Write a SARIF 2.1.0 report (used by GitHub Code Scanning) |

### Acknowledgement file (`--ack`)

```json
{
  "acknowledged": [
    { "account": "Vault", "field": "legacy_data", "change": "removed", "note": "intentional cleanup" }
  ]
}
```

### Hints file (`--hints`)

Use when a field is renamed **and** moved to a different position simultaneously (auto-detection requires same position):

```json
{
  "renames": [
    { "account": "Vault", "from": "amount", "to": "balance" }
  ]
}
```

## GitHub Action

```yaml
- name: Check Vault layout
  uses: Rudraprajapati2612/layoutd-cli@v0.1.0
  with:
    old-idl: old_idl.json
    new-idl: idl.json
    account: Vault
    sarif-output: layoutd.sarif
    zero-copy: 'false'       # optional
    ack-file: layoutd.ack    # optional
    hints-file: hints.json   # optional
```

## Crates

| Crate | Description |
|---|---|
| [`layoutd`](https://crates.io/crates/layoutd) | CLI binary (`cargo install layoutd`) |
| [`layoutd-core`](https://crates.io/crates/layoutd-core) | Core library (diff engine, classifier, SARIF emitter) |

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
