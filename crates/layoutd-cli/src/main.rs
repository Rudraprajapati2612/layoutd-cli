mod ack;
mod sarif;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use layoutd_core::borsh::compute_layout;
use layoutd_core::classify::{classify_all, ClassifiedChange, Safety};
use layoutd_core::diff::{diff, ChangeKind};
use layoutd_core::idl::{parse_idl, FieldType};
use layoutd_core::zerocopy::{classify_zc_all, compute_zc_layout, zc_to_borsh_layout};

#[derive(Parser)]
#[command(name = "layoutd", about = "Solana account layout migration safety tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print every field change with Safe / Review / Danger tags
    Diff {
        old: PathBuf,
        new: PathBuf,
        #[arg(long)]
        account: String,
        /// Analyse as a repr(C) zero-copy account (stricter rules)
        #[arg(long)]
        zero_copy: bool,
    },
    /// Generate migration code; scaffold dangerous changes
    Gen {
        old: PathBuf,
        new: PathBuf,
        #[arg(long)]
        account: String,
        /// Analyse as a repr(C) zero-copy account (stricter rules)
        #[arg(long)]
        zero_copy: bool,
    },
    /// Exit 1 if any Danger change found (CI gate)
    Check {
        old: PathBuf,
        new: PathBuf,
        #[arg(long)]
        account: String,
        /// Write a SARIF 2.1.0 report to this path (enables GitHub PR annotations)
        #[arg(long)]
        sarif: Option<PathBuf>,
        /// Analyse as a repr(C) zero-copy account (stricter rules)
        #[arg(long)]
        zero_copy: bool,
        /// Path to a layoutd.ack file; named dangers are allowed through CI
        #[arg(long)]
        ack: Option<PathBuf>,
    },
}

// ── shared pipeline ───────────────────────────────────────────────────────────

fn run_pipeline(old: &PathBuf, new: &PathBuf, account: &str) -> Vec<ClassifiedChange> {
    let old_def = parse_idl(old, account).unwrap_or_else(|e| {
        eprintln!("error reading old IDL: {e}");
        std::process::exit(1);
    });
    let new_def = parse_idl(new, account).unwrap_or_else(|e| {
        eprintln!("error reading new IDL: {e}");
        std::process::exit(1);
    });
    classify_all(diff(&compute_layout(&old_def), &compute_layout(&new_def)))
}

fn run_zc_pipeline(old: &PathBuf, new: &PathBuf, account: &str) -> Vec<ClassifiedChange> {
    let old_def = parse_idl(old, account).unwrap_or_else(|e| {
        eprintln!("error reading old IDL: {e}");
        std::process::exit(1);
    });
    let new_def = parse_idl(new, account).unwrap_or_else(|e| {
        eprintln!("error reading new IDL: {e}");
        std::process::exit(1);
    });
    let old_zc = compute_zc_layout(&old_def).unwrap_or_else(|e| {
        eprintln!("zero-copy layout error (old IDL): {e}");
        std::process::exit(1);
    });
    let new_zc = compute_zc_layout(&new_def).unwrap_or_else(|e| {
        eprintln!("zero-copy layout error (new IDL): {e}");
        std::process::exit(1);
    });
    let changes = diff(&zc_to_borsh_layout(&old_zc), &zc_to_borsh_layout(&new_zc));
    classify_zc_all(changes, &old_zc, &new_zc)
}

// ── diff command ──────────────────────────────────────────────────────────────

fn cmd_diff(old: &PathBuf, new: &PathBuf, account: &str, zero_copy: bool) {
    let classified = if zero_copy {
        run_zc_pipeline(old, new, account)
    } else {
        run_pipeline(old, new, account)
    };

    let mode = if zero_copy { "zero-copy" } else { "borsh" };
    println!("layoutd diff  —  account: {account}  [{mode}]");
    println!("{}", "─".repeat(90));
    println!("{:<26} {:<30} {:<8} {}", "FIELD", "CHANGE", "SAFETY", "REASON");
    println!("{}", "─".repeat(90));

    for c in &classified {
        println!(
            "{:<26} {:<30} {:<8} {}",
            c.change.name,
            format_change_kind(&c.change.kind),
            safety_str(&c.safety),
            c.reason,
        );
    }

    println!("{}", "─".repeat(90));

    let n_safe   = classified.iter().filter(|c| c.safety == Safety::Safe).count();
    let n_review = classified.iter().filter(|c| c.safety == Safety::Review).count();
    let n_danger = classified.iter().filter(|c| c.safety == Safety::Danger).count();
    println!("  {n_safe} safe   {n_review} review   {n_danger} danger");
}

fn format_change_kind(kind: &ChangeKind) -> String {
    match kind {
        ChangeKind::Unchanged => "unchanged".to_string(),
        ChangeKind::Added { at_index } => format!("added at index {at_index}"),
        ChangeKind::Removed { from_index } => format!("removed (was index {from_index})"),
        ChangeKind::Renamed { from_name } => format!("renamed from '{from_name}'"),
        ChangeKind::TypeChanged { old_type, new_type } => {
            format!("{} → {}", type_name(old_type), type_name(new_type))
        }
        ChangeKind::Reordered { old_index, new_index } => {
            format!("reordered {old_index} → {new_index}")
        }
        ChangeKind::TypeChangedAndReordered { old_type, old_index, new_type, new_index } => {
            format!("{}@{old_index} → {}@{new_index}", type_name(old_type), type_name(new_type))
        }
    }
}

fn safety_str(s: &Safety) -> &'static str {
    match s {
        Safety::Safe   => "SAFE",
        Safety::Review => "REVIEW",
        Safety::Danger => "DANGER",
    }
}

// ── gen command ───────────────────────────────────────────────────────────────

fn cmd_gen(old: &PathBuf, new: &PathBuf, account: &str, zero_copy: bool) {
    let classified = if zero_copy {
        run_zc_pipeline(old, new, account)
    } else {
        run_pipeline(old, new, account)
    };

    let has_danger = classified.iter().any(|c| c.safety == Safety::Danger);
    let mode = if zero_copy { "zero-copy" } else { "borsh" };

    println!("// layoutd gen  —  account: {account}  [{mode}]");
    if has_danger {
        println!("// WARNING: dangerous changes present — resolve every DANGER line before shipping");
    }
    println!();
    println!("impl Migration<Old{account}, {account}> {{");
    println!("    pub fn migrate(old: Old{account}) -> {account} {{");
    println!("        {account} {{");

    for c in &classified {
        let field = &c.change.name;
        let line = gen_field_line(field, &c.change.kind, &c.safety, c.reason);
        println!("{line}");
    }

    println!("        }}");
    println!("    }}");
    println!("}}");
}

fn gen_field_line(field: &str, kind: &ChangeKind, safety: &Safety, reason: &str) -> String {
    match kind {
        ChangeKind::Unchanged | ChangeKind::Reordered { .. } => {
            format!("            {field}: old.{field},")
        }

        ChangeKind::Renamed { from_name } => {
            format!("            {field}: old.{from_name},")
        }

        ChangeKind::Added { .. } => match safety {
            Safety::Safe   => format!("            {field}: Default::default(),"),
            Safety::Review => format!("            {field}: Default::default(), // REVIEW: {reason}"),
            Safety::Danger => format!("            // DANGER: {field} — {reason}"),
        },

        ChangeKind::Removed { .. } => {
            format!("            // DANGER: '{field}' removed — {reason} — decide how to handle lost data")
        }

        ChangeKind::TypeChanged { old_type, new_type } => match safety {
            Safety::Safe => format!("            {field}: old.{field},"),
            Safety::Review => {
                let expr = cast_expr(field, old_type, new_type);
                format!("            {field}: {expr}, // REVIEW: {reason}")
            }
            Safety::Danger => format!(
                "            // DANGER: {field} ({} → {}) — {reason}\n            // {field}: todo!(\"supply conversion\"),",
                type_name(old_type),
                type_name(new_type),
            ),
        },

        ChangeKind::TypeChangedAndReordered { old_type, new_type, .. } => match safety {
            Safety::Safe => format!("            {field}: old.{field},"),
            Safety::Review => {
                let expr = cast_expr(field, old_type, new_type);
                format!("            {field}: {expr}, // REVIEW: reordered + {reason}")
            }
            Safety::Danger => format!(
                "            // DANGER: {field} ({} → {}, reordered) — {reason}\n            // {field}: todo!(\"supply conversion\"),",
                type_name(old_type),
                type_name(new_type),
            ),
        },
    }
}

fn cast_expr(field: &str, old_ty: &FieldType, new_ty: &FieldType) -> String {
    match (old_ty, new_ty) {
        (FieldType::F32, FieldType::F64) => format!("f64::from(old.{field})"),
        _ => format!("old.{field} as {}", type_name(new_ty)),
    }
}

// ── check command ─────────────────────────────────────────────────────────────

fn cmd_check(
    old: &PathBuf,
    new: &PathBuf,
    account: &str,
    sarif_out: Option<&PathBuf>,
    zero_copy: bool,
    ack_path: Option<&PathBuf>,
) {
    let classified = if zero_copy {
        run_zc_pipeline(old, new, account)
    } else {
        run_pipeline(old, new, account)
    };

    // Load acknowledgements if provided.
    let acks: Vec<ack::Ack> = if let Some(path) = ack_path {
        ack::load(path).unwrap_or_else(|e| {
            eprintln!("error loading ack file: {e}");
            std::process::exit(1);
        })
    } else {
        Vec::new()
    };

    let ack_result = ack::check(account, &classified, &acks);

    // Write SARIF before any exit so the file is always present for upload-sarif.
    if let Some(path) = sarif_out {
        let doc = sarif::build(&classified, new, zero_copy);
        let json = serde_json::to_string_pretty(&doc).expect("SARIF serialization failed");
        std::fs::write(path, json).unwrap_or_else(|e| {
            eprintln!("error writing SARIF to '{}': {e}", path.display());
            std::process::exit(1);
        });
        eprintln!("SARIF report written to {}", path.display());
    }

    let mode = if zero_copy { "zero-copy" } else { "borsh" };
    let reviews: Vec<_> = classified.iter().filter(|c| c.safety == Safety::Review).collect();

    if !reviews.is_empty() {
        println!(
            "layoutd check: {} REVIEW item(s) for {account} [{mode}]",
            reviews.len()
        );
        for c in &reviews {
            println!("  REVIEW  {}  —  {}", c.change.name, c.reason);
        }
    }

    // Report acknowledged dangers (allowed through, but still visible).
    for &di in &ack_result.acknowledged {
        let c = &classified[di];
        // Find the matching ack to show its note.
        let note = acks.iter()
            .find(|a| a.field == c.change.name)
            .map(|a| a.note.as_str())
            .unwrap_or("");
        if note.is_empty() {
            println!("  ACK     {}  —  acknowledged danger, CI pass allowed", c.change.name);
        } else {
            println!("  ACK     {}  —  acknowledged: {note}", c.change.name);
        }
    }

    // Stale acks: were in the file but matched no actual danger.
    for stale in &ack_result.stale {
        println!(
            "  WARN    ack '{}:{}' ({}) has no matching danger — remove or update it",
            stale.account, stale.field, stale.change
        );
    }

    if ack_result.unacknowledged.is_empty() {
        let total = classified.len();
        let n_acked = ack_result.acknowledged.len();
        if n_acked > 0 {
            println!(
                "layoutd check: OK — {total} changes for {account} [{mode}], {n_acked} danger(s) acknowledged"
            );
        } else if reviews.is_empty() {
            println!(
                "layoutd check: OK — all {total} changes safe for {account} [{mode}]"
            );
        }
        std::process::exit(0);
    }

    // Unacknowledged dangers → fail.
    println!(
        "layoutd check: FAIL — {} unacknowledged dangerous change(s) in {account} [{mode}]",
        ack_result.unacknowledged.len()
    );
    for &di in &ack_result.unacknowledged {
        let c = &classified[di];
        println!("  DANGER  {}  —  {}", c.change.name, c.reason);
    }
    println!();
    println!("Run `layoutd gen` to see a scaffold with every DANGER annotated.");
    if ack_path.is_some() {
        println!("Add an entry to your ack file to acknowledge this danger deliberately.");
    } else {
        println!("Use --ack <file> to acknowledge deliberate dangerous changes.");
    }
    std::process::exit(1);
}

// ── type name helper ──────────────────────────────────────────────────────────

fn type_name(ty: &FieldType) -> String {
    match ty {
        FieldType::U8    => "u8".to_string(),
        FieldType::U16   => "u16".to_string(),
        FieldType::U32   => "u32".to_string(),
        FieldType::U64   => "u64".to_string(),
        FieldType::U128  => "u128".to_string(),
        FieldType::I8    => "i8".to_string(),
        FieldType::I16   => "i16".to_string(),
        FieldType::I32   => "i32".to_string(),
        FieldType::I64   => "i64".to_string(),
        FieldType::I128  => "i128".to_string(),
        FieldType::Bool  => "bool".to_string(),
        FieldType::F32   => "f32".to_string(),
        FieldType::F64   => "f64".to_string(),
        FieldType::Pubkey => "Pubkey".to_string(),
        FieldType::String => "String".to_string(),
        FieldType::Vec(inner)      => format!("Vec<{}>", type_name(inner)),
        FieldType::Array(inner, n) => format!("[{}; {n}]", type_name(inner)),
        FieldType::Option(inner)   => format!("Option<{}>", type_name(inner)),
        FieldType::Defined(name)   => name.clone(),
        FieldType::Unknown(raw)    => format!("/* unknown: {raw} */"),
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Command::Diff  { old, new, account, zero_copy }        => cmd_diff(old, new, account, *zero_copy),
        Command::Gen   { old, new, account, zero_copy }        => cmd_gen(old, new, account, *zero_copy),
        Command::Check { old, new, account, sarif, zero_copy, ack } => {
            cmd_check(old, new, account, sarif.as_ref(), *zero_copy, ack.as_ref())
        }
    }
}
