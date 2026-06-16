use std::path::Path;

use syn::{
    Expr, File, GenericArgument, Item, Lit, PathArguments, Type,
};

use crate::idl::{AccountDef, FieldDef, FieldType};

/// Parse a Rust source file and extract an account struct definition.
///
/// Walks all top-level and module-nested items looking for a `struct`
/// whose name matches `account_name`. Attributes (e.g. `#[account(zero_copy)]`,
/// `#[repr(C)]`) are ignored — only the field list matters.
///
/// Returns `Err` when the file cannot be read, the Rust syntax is invalid,
/// or no matching struct is found.
pub fn parse_source(path: &Path, account_name: &str) -> Result<AccountDef, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
    parse_source_str(&content, account_name)
        .map_err(|e| format!("{e} (in '{}')", path.display()))
}

/// Parse from an in-memory string — useful in tests without touching the fs.
pub fn parse_source_str(src: &str, account_name: &str) -> Result<AccountDef, String> {
    let file: File =
        syn::parse_str(src).map_err(|e| format!("Rust parse error: {e}"))?;
    find_struct(&file.items, account_name)
        .ok_or_else(|| format!("struct '{account_name}' not found in source"))?
}

fn find_struct(items: &[Item], account_name: &str) -> Option<Result<AccountDef, String>> {
    for item in items {
        match item {
            Item::Struct(s) if s.ident == account_name => {
                return Some(extract_def(s));
            }
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    if let Some(r) = find_struct(inner, account_name) {
                        return Some(r);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_def(s: &syn::ItemStruct) -> Result<AccountDef, String> {
    let syn::Fields::Named(fields) = &s.fields else {
        return Err(format!(
            "struct '{}' has no named fields — only named-field structs are supported",
            s.ident
        ));
    };

    let parsed: Result<Vec<FieldDef>, String> = fields
        .named
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let name = f.ident.as_ref().unwrap().to_string();
            let ty = syn_to_field_type(&f.ty)
                .map_err(|e| format!("field '{name}': {e}"))?;
            Ok(FieldDef { name, ty, index: i })
        })
        .collect();

    Ok(AccountDef {
        name: s.ident.to_string(),
        fields: parsed?,
    })
}

pub fn syn_to_field_type(ty: &Type) -> Result<FieldType, String> {
    match ty {
        Type::Path(tp) => {
            let seg = tp
                .path
                .segments
                .last()
                .ok_or_else(|| "empty type path".to_string())?;
            let ident = seg.ident.to_string();

            // Primitive / known named types
            match ident.as_str() {
                "u8"     => return Ok(FieldType::U8),
                "u16"    => return Ok(FieldType::U16),
                "u32"    => return Ok(FieldType::U32),
                "u64"    => return Ok(FieldType::U64),
                "u128"   => return Ok(FieldType::U128),
                "i8"     => return Ok(FieldType::I8),
                "i16"    => return Ok(FieldType::I16),
                "i32"    => return Ok(FieldType::I32),
                "i64"    => return Ok(FieldType::I64),
                "i128"   => return Ok(FieldType::I128),
                "bool"   => return Ok(FieldType::Bool),
                "f32"    => return Ok(FieldType::F32),
                "f64"    => return Ok(FieldType::F64),
                "Pubkey" => return Ok(FieldType::Pubkey),
                "String" => return Ok(FieldType::String),
                _ => {}
            }

            // Parameterised types
            match ident.as_str() {
                "Vec" => {
                    let inner = single_angle_arg(&seg.arguments, "Vec")?;
                    Ok(FieldType::Vec(Box::new(inner)))
                }
                "Option" => {
                    let inner = single_angle_arg(&seg.arguments, "Option")?;
                    Ok(FieldType::Option(Box::new(inner)))
                }
                other => Ok(FieldType::Defined(other.to_string())),
            }
        }

        Type::Array(ta) => {
            let inner = syn_to_field_type(&ta.elem)?;
            let len = array_len(&ta.len)?;
            Ok(FieldType::Array(Box::new(inner), len))
        }

        // Everything else (references, raw pointers, fn types, …) → Unknown
        other => Ok(FieldType::Unknown(format!("{}", quote_type(other)))),
    }
}

fn single_angle_arg(args: &PathArguments, parent: &str) -> Result<FieldType, String> {
    let PathArguments::AngleBracketed(ab) = args else {
        return Err(format!("{parent}<T> missing angle-bracket arguments"));
    };
    let Some(GenericArgument::Type(inner)) = ab.args.first() else {
        return Err(format!("{parent}<T> has no type argument"));
    };
    syn_to_field_type(inner)
}

fn array_len(expr: &Expr) -> Result<usize, String> {
    match expr {
        Expr::Lit(el) => match &el.lit {
            Lit::Int(li) => li
                .base10_parse::<usize>()
                .map_err(|_| "array length too large".to_string()),
            _ => Err("array length must be an integer literal".to_string()),
        },
        _ => Err("array length must be a constant integer literal".to_string()),
    }
}

fn quote_type(ty: &Type) -> String {
    // Best-effort display — syn types don't implement Display directly.
    use std::fmt::Write;
    let mut s = String::new();
    match ty {
        Type::Reference(_) => write!(s, "&<type>").unwrap(),
        Type::Ptr(_)       => write!(s, "*<type>").unwrap(),
        _                  => write!(s, "<unsupported>").unwrap(),
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── primitive types ────────────────────────────────────────────────────────

    #[test]
    fn all_integer_types() {
        let src = r#"
            pub struct Vault {
                pub a: u8,  pub b: u16, pub c: u32, pub d: u64, pub e: u128,
                pub f: i8,  pub g: i16, pub h: i32, pub i: i64, pub j: i128,
            }
        "#;
        let def = parse_source_str(src, "Vault").unwrap();
        let types: Vec<_> = def.fields.iter().map(|f| &f.ty).collect();
        assert!(matches!(types[0], FieldType::U8));
        assert!(matches!(types[1], FieldType::U16));
        assert!(matches!(types[2], FieldType::U32));
        assert!(matches!(types[3], FieldType::U64));
        assert!(matches!(types[4], FieldType::U128));
        assert!(matches!(types[5], FieldType::I8));
        assert!(matches!(types[6], FieldType::I16));
        assert!(matches!(types[7], FieldType::I32));
        assert!(matches!(types[8], FieldType::I64));
        assert!(matches!(types[9], FieldType::I128));
    }

    #[test]
    fn bool_float_pubkey_string() {
        let src = r#"
            pub struct S {
                pub flag: bool,
                pub x: f32,
                pub y: f64,
                pub owner: Pubkey,
                pub label: String,
            }
        "#;
        let def = parse_source_str(src, "S").unwrap();
        assert!(matches!(def.fields[0].ty, FieldType::Bool));
        assert!(matches!(def.fields[1].ty, FieldType::F32));
        assert!(matches!(def.fields[2].ty, FieldType::F64));
        assert!(matches!(def.fields[3].ty, FieldType::Pubkey));
        assert!(matches!(def.fields[4].ty, FieldType::String));
    }

    // ── compound types ─────────────────────────────────────────────────────────

    #[test]
    fn vec_of_u8() {
        let src = r#"pub struct S { pub data: Vec<u8> }"#;
        let def = parse_source_str(src, "S").unwrap();
        assert!(matches!(&def.fields[0].ty, FieldType::Vec(inner) if **inner == FieldType::U8));
    }

    #[test]
    fn option_of_pubkey() {
        let src = r#"pub struct S { pub owner: Option<Pubkey> }"#;
        let def = parse_source_str(src, "S").unwrap();
        assert!(
            matches!(&def.fields[0].ty, FieldType::Option(inner) if **inner == FieldType::Pubkey)
        );
    }

    #[test]
    fn array_of_u8_32() {
        let src = r#"pub struct S { pub id: [u8; 32] }"#;
        let def = parse_source_str(src, "S").unwrap();
        assert!(
            matches!(&def.fields[0].ty, FieldType::Array(inner, 32) if **inner == FieldType::U8)
        );
    }

    #[test]
    fn array_of_u64_4() {
        let src = r#"pub struct S { pub keys: [u64; 4] }"#;
        let def = parse_source_str(src, "S").unwrap();
        assert!(
            matches!(&def.fields[0].ty, FieldType::Array(inner, 4) if **inner == FieldType::U64)
        );
    }

    #[test]
    fn nested_vec_of_pubkey() {
        let src = r#"pub struct S { pub members: Vec<Pubkey> }"#;
        let def = parse_source_str(src, "S").unwrap();
        assert!(
            matches!(&def.fields[0].ty, FieldType::Vec(inner) if **inner == FieldType::Pubkey)
        );
    }

    #[test]
    fn user_defined_type_becomes_defined() {
        let src = r#"pub struct S { pub state: MarketState }"#;
        let def = parse_source_str(src, "S").unwrap();
        assert!(matches!(&def.fields[0].ty, FieldType::Defined(n) if n == "MarketState"));
    }

    // ── attributes are ignored ─────────────────────────────────────────────────

    #[test]
    fn account_attributes_are_ignored() {
        let src = r#"
            #[account(zero_copy)]
            #[repr(C)]
            pub struct VaultZC {
                pub owner: Pubkey,
                pub balance: u64,
                pub bump: u8,
            }
        "#;
        let def = parse_source_str(src, "VaultZC").unwrap();
        assert_eq!(def.name, "VaultZC");
        assert_eq!(def.fields.len(), 3);
        assert!(matches!(def.fields[0].ty, FieldType::Pubkey));
        assert!(matches!(def.fields[1].ty, FieldType::U64));
        assert!(matches!(def.fields[2].ty, FieldType::U8));
    }

    // ── field indices ──────────────────────────────────────────────────────────

    #[test]
    fn field_indices_are_assigned_in_order() {
        let src = r#"
            pub struct S { pub a: u8, pub b: u16, pub c: u32 }
        "#;
        let def = parse_source_str(src, "S").unwrap();
        for (i, f) in def.fields.iter().enumerate() {
            assert_eq!(f.index, i, "field '{}' has wrong index", f.name);
        }
    }

    // ── error paths ────────────────────────────────────────────────────────────

    #[test]
    fn struct_not_found_returns_error() {
        let src = r#"pub struct Other { pub x: u8 }"#;
        let err = parse_source_str(src, "Missing").unwrap_err();
        assert!(err.contains("Missing"), "{err}");
    }

    #[test]
    fn bad_rust_syntax_returns_error() {
        let err = parse_source_str("not valid rust !!!!", "S").unwrap_err();
        assert!(err.contains("parse error") || err.contains("parse"), "{err}");
    }

    #[test]
    fn struct_inside_module_is_found() {
        let src = r#"
            mod inner {
                pub struct Hidden { pub x: u64 }
            }
        "#;
        let def = parse_source_str(src, "Hidden").unwrap();
        assert_eq!(def.name, "Hidden");
        assert_eq!(def.fields.len(), 1);
    }

    #[test]
    fn tuple_struct_returns_error() {
        let src = r#"pub struct Tuple(u8, u64);"#;
        let err = parse_source_str(src, "Tuple").unwrap_err();
        assert!(err.contains("named fields") || err.contains("Tuple"), "{err}");
    }

    #[test]
    fn missing_file_returns_error() {
        let err = parse_source(Path::new("/no/such/file.rs"), "S").unwrap_err();
        assert!(err.contains("/no/such/file.rs"), "{err}");
    }

    // ── roundtrip with ZC layout ───────────────────────────────────────────────

    #[test]
    fn source_to_zc_layout_matches_known_offsets() {
        use crate::zerocopy::compute_zc_layout;

        let src = r#"
            #[account(zero_copy)]
            pub struct VaultZC {
                pub owner:   Pubkey,   // 32 bytes, align 1 → offset 8
                pub balance: u64,      // 8 bytes,  align 8 → offset 40
                pub bump:    u8,       // 1 byte,   align 1 → offset 48
            }
        "#;
        let def = parse_source_str(src, "VaultZC").unwrap();
        let zc = compute_zc_layout(&def).unwrap();

        assert_eq!(zc.fields[0].offset, 8,  "owner offset");
        assert_eq!(zc.fields[1].offset, 40, "balance offset");
        assert_eq!(zc.fields[2].offset, 48, "bump offset");
        assert_eq!(zc.trailing_padding, 7);
        assert_eq!(zc.total_size, 56);
    }
}
