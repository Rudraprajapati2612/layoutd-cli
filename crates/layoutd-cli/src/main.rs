use std::path::PathBuf;

use clap::{Parser, Subcommand};
use layoutd_core::borsh::compute_layout;
use layoutd_core::classify::{classify_all, ClassifiedChange, Safety};
use layoutd_core::diff::{diff, ChangeKind};
use layoutd_core::idl::{parse_idl, FieldType};

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
    },
    /// Generate migration code; scaffold dangerous changes
    Gen {
        old: PathBuf,
        new: PathBuf,
        #[arg(long)]
        account: String,
    },
    /// Exit 1 if any Danger change found (CI gate)
    Check {
        old: PathBuf,
        new: PathBuf,
        #[arg(long)]
        account: String,
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

// ── diff command ──────────────────────────────────────────────────────────────

fn cmd_diff(old: &PathBuf, new: &PathBuf, account: &str) {
    let classified = run_pipeline(old, new, account);

    println!("layoutd diff  —  account: {account}");
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

fn cmd_gen(old: &PathBuf, new: &PathBuf, account: &str) {
    let classified = run_pipeline(old, new, account);
    let has_danger = classified.iter().any(|c| c.safety == Safety::Danger);

    println!("// layoutd gen  —  account: {account}");
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
        // Carry the value straight across
        ChangeKind::Unchanged | ChangeKind::Reordered { .. } => {
            format!("            {field}: old.{field},")
        }

        ChangeKind::Renamed { from_name } => {
            format!("            {field}: old.{from_name},")
        }

        // New field — use Default, annotate when review is needed
        ChangeKind::Added { .. } => match safety {
            Safety::Safe   => format!("            {field}: Default::default(),"),
            Safety::Review => format!("            {field}: Default::default(), // REVIEW: {reason}"),
            Safety::Danger => format!("            // DANGER: {field} — {reason}"),
        },

        // Removed field — always Danger; leave a scaffold comment
        ChangeKind::Removed { .. } => {
            format!("            // DANGER: '{field}' removed — {reason} — decide how to handle lost data")
        }

        // Type change only
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

        // Type change + reorder — safety is driven by the type change
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

// Produce a cast expression for Review-level widening.
fn cast_expr(field: &str, old_ty: &FieldType, new_ty: &FieldType) -> String {
    match (old_ty, new_ty) {
        (FieldType::F32, FieldType::F64) => format!("f64::from(old.{field})"),
        _ => format!("old.{field} as {}", type_name(new_ty)),
    }
}

// ── check command ─────────────────────────────────────────────────────────────

fn cmd_check(old: &PathBuf, new: &PathBuf, account: &str) {
    let classified = run_pipeline(old, new, account);

    let dangers: Vec<_> = classified.iter().filter(|c| c.safety == Safety::Danger).collect();
    let reviews: Vec<_> = classified.iter().filter(|c| c.safety == Safety::Review).collect();

    if dangers.is_empty() && reviews.is_empty() {
        println!("layoutd check: OK — all {} changes safe for {account}", classified.len());
        std::process::exit(0);
    }

    if !reviews.is_empty() {
        println!("layoutd check: {} REVIEW item(s) for {account}", reviews.len());
        for c in &reviews {
            println!("  REVIEW  {}  —  {}", c.change.name, c.reason);
        }
    }

    if !dangers.is_empty() {
        println!(
            "layoutd check: FAIL — {} dangerous change(s) in {account}",
            dangers.len()
        );
        for c in &dangers {
            println!("  DANGER  {}  —  {}", c.change.name, c.reason);
        }
        println!();
        println!("Run `layoutd gen` to see a scaffold with every DANGER annotated.");
        std::process::exit(1);
    }

    // Reviews present but no Danger → CI passes, developer sees warnings above
    std::process::exit(0);
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
        FieldType::Vec(inner)       => format!("Vec<{}>", type_name(inner)),
        FieldType::Array(inner, n)  => format!("[{}; {n}]", type_name(inner)),
        FieldType::Option(inner)    => format!("Option<{}>", type_name(inner)),
        FieldType::Defined(name)    => name.clone(),
        FieldType::Unknown(raw)     => format!("/* unknown: {raw} */"),
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Command::Diff  { old, new, account } => cmd_diff(old, new, account),
        Command::Gen   { old, new, account } => cmd_gen(old, new, account),
        Command::Check { old, new, account } => cmd_check(old, new, account),
    }
}
