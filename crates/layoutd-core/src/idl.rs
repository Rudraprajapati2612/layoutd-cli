use serde::Deserialize;
use std::{ path::Path};

#[derive(Debug, Clone, PartialEq)]
pub struct  AccountDef{
    pub name : String,
    pub fields : Vec<FieldDef>
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef{
    pub name : String,
    pub ty : FieldType,
    pub index : usize
}

#[derive(Debug, Clone, PartialEq)]
pub enum  FieldType {
    U8, U16, U32, U64, U128,
    I8, I16, I32, I64, I128,
    Bool,F32,F64,
    Pubkey,
    String,
    Vec(Box<FieldType>),
    Array(Box<FieldType>, usize),
    Option(Box<FieldType>),
    Defined(String),
    Unknown(String),
}

#[derive(Deserialize)]
struct  RawFields{
    name : String,
    // it map the anchor idl in that value we get from the type that store it into ty in rust struct 
    #[serde(rename="type")]
    ty : serde_json::Value
}

#[derive(Deserialize)]
struct RawAccountType{
    kind: String,
    #[serde(default)]
    fields : Vec<RawFields>
}
#[derive(Deserialize)]
struct  RawAccount{
     name: String,

    #[serde(rename = "type")]
    ty: RawAccountType,
}

#[derive(Deserialize)]
struct RawIdl {
    types: Vec<RawAccount>,
}

pub fn parse_idl(path : &Path,account_name : &str) -> Result<AccountDef,String>{
    // read the file 
    let content = std::fs::read_to_string(path).map_err(|e| format!("failed to read IDL at '{}': {}", path.display(), e))?;

    // deserialize(Converting a json formate into a rust struct) a raw idl into 

    let raw:RawIdl = serde_json::from_str(&content).map_err(|e| format!("Failed to parse Idl: {e}"))?;

    // Find the accountes by types  

    let accounts = raw.types.into_iter().find(|a| a.name == account_name).ok_or_else(|| format!("Account '{}' not found in IDL", account_name))?;

    if accounts.ty.kind != "struct" {
        return Err(format!("'{}' is a {} not a struct — only struct accounts have a byte layout", account_name, accounts.ty.kind));
    }
    // it iterate over fields section in idl json and store all the details in fieldDef struct 
    // like at index 1 at idl we have name = "market" and   type = "pubkey" then 
    // FieldDef struct will look like FieldDef{name : "market".to_string() , type : FieldType::Pubkey, index : 0}
    let field = accounts.ty.fields.into_iter().enumerate().map(|(index,raw_field)|FieldDef{
        name : raw_field.name,
        ty : parse_field_type(&raw_field.ty),
        index 
    }).collect();

    Ok(
        AccountDef{
            name : account_name.to_string(),
            fields : field
        }
    )
}


pub fn parse_field_type(value:&serde_json::Value) -> FieldType{

    match value{
        serde_json::Value::String(s) => match s.as_str(){
            "u8"        => FieldType::U8,
            "u16"       => FieldType::U16,
            "u32"       => FieldType::U32,
            "u64"       => FieldType::U64,
            "u128"      => FieldType::U128,
            "i8"        => FieldType::I8,
            "i16"       => FieldType::I16,
            "i32"       => FieldType::I32,
            "i64"       => FieldType::I64,
            "i128"      => FieldType::I128,
            "bool"      => FieldType::Bool,
            "f32"       => FieldType::F32,
            "f64"       => FieldType::F64,
            "pubkey"    => FieldType::Pubkey,
            "string"    => FieldType::String,
            other       => FieldType::Unknown(other.to_string()),
        },
        serde_json::Value::Object(map) => {
            if let Some(inner) = map.get("vec") {
                FieldType::Vec(Box::new(parse_field_type(inner)))
            } else if let Some(arr) = map.get("array") {
                // array is ["u64", 10] — a JSON array with type + length
                let Some(inner) = arr.get(0) else {
                    return FieldType::Unknown(format!("{:?}", arr));
                };
                let Some(len_value) = arr.get(1) else {
                    return FieldType::Unknown(format!("{:?}", arr));
                };
                let inner_ty = parse_field_type(inner);
                let len = len_value.as_u64().unwrap_or(0) as usize;
                FieldType::Array(Box::new(inner_ty), len)
            } else if let Some(inner) = map.get("option") {
                FieldType::Option(Box::new(parse_field_type(inner)))
            } else if let Some(defined) = map.get("defined") {
                let name = defined.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                FieldType::Defined(name)
            } else {
                FieldType::Unknown(format!("{:?}", map))
            }
        },
        other => FieldType::Unknown(format!("{:?}", other)),

    }
}





// Tests 
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn idl_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../idl.json")
    }

    // EscrowVault Test 

    #[test]
    fn escrow_vault_parses_successfully() {
        parse_idl(&idl_path(), "EscrowVault").unwrap();
    }

    #[test]
    fn escrow_vault_name() {
        let def = parse_idl(&idl_path(), "EscrowVault").unwrap();
        assert_eq!(def.name, "EscrowVault");
    }

    #[test]
    fn escrow_vault_field_count() {
        let def = parse_idl(&idl_path(), "EscrowVault").unwrap();
        assert_eq!(def.fields.len(), 12);
    }

    #[test]
    fn escrow_vault_field_order_matches_indices() {
        let def = parse_idl(&idl_path(), "EscrowVault").unwrap();
        for (i, field) in def.fields.iter().enumerate() {
            assert_eq!(field.index, i, "field '{}' has wrong index", field.name);
        }
    }

    #[test]
    fn escrow_vault_all_fields() {
        let def = parse_idl(&idl_path(), "EscrowVault").unwrap();

        let expected: &[(&str, FieldType)] = &[
            ("market",                    FieldType::Pubkey),
            ("mrarket_registery_program", FieldType::Pubkey),
            ("usdc_vault",                FieldType::Pubkey),
            ("yes_token_mint",            FieldType::Pubkey),
            ("no_token_mint",             FieldType::Pubkey),
            ("total_locked_collateral",   FieldType::U64),
            ("total_yes_minted",          FieldType::U64),
            ("total_no_minted",           FieldType::U64),
            ("is_settled",                FieldType::Bool),
            ("is_minting_paused",         FieldType::Bool),
            ("admin",                     FieldType::Pubkey),
            ("bump",                      FieldType::U8),
        ];

        assert_eq!(def.fields.len(), expected.len());
        for (field, (exp_name, exp_ty)) in def.fields.iter().zip(expected.iter()) {
            assert_eq!(&field.name, exp_name, "field name mismatch at index {}", field.index);
            assert_eq!(&field.ty,   exp_ty,   "field type mismatch for '{}'",    field.name);
        }
    }

    // Market Test 

    #[test]
    fn market_parses_successfully() {
        parse_idl(&idl_path(), "Market").unwrap();
    }

    #[test]
    fn market_field_count() {
        let def = parse_idl(&idl_path(), "Market").unwrap();
        assert_eq!(def.fields.len(), 16);
    }

    #[test]
    fn market_field_order_matches_indices() {
        let def = parse_idl(&idl_path(), "Market").unwrap();
        for (i, field) in def.fields.iter().enumerate() {
            assert_eq!(field.index, i, "field '{}' has wrong index", field.name);
        }
    }

    #[test]
    fn market_all_fields() {
        let def = parse_idl(&idl_path(), "Market").unwrap();

        let expected: &[(&str, FieldType)] = &[
            ("market_id",          FieldType::Array(Box::new(FieldType::U8), 32)),
            ("question",           FieldType::String),
            ("description",        FieldType::String),
            ("category",           FieldType::String),
            ("creator",            FieldType::Pubkey),
            ("created_at",         FieldType::I64),
            ("expire_at",          FieldType::I64),
            ("state",              FieldType::Defined("MarketState".to_string())),
            ("yes_token_mint",     FieldType::Pubkey),
            ("no_token_mint",      FieldType::Pubkey),
            ("escrow_vault",       FieldType::Pubkey),
            ("resolution_adapter", FieldType::Pubkey),
            ("resolution_source",  FieldType::String),
            ("resolution_outcome", FieldType::Option(Box::new(FieldType::Defined("ResultOutcome".to_string())))),
            ("resolved_at",        FieldType::Option(Box::new(FieldType::I64))),
            ("bump",               FieldType::U8),
        ];

        assert_eq!(def.fields.len(), expected.len());
        for (field, (exp_name, exp_ty)) in def.fields.iter().zip(expected.iter()) {
            assert_eq!(&field.name, exp_name, "field name mismatch at index {}", field.index);
            assert_eq!(&field.ty,   exp_ty,   "field type mismatch for '{}'",    field.name);
        }
    }

    // Error paths Test 

    #[test]
    fn unknown_account_name_returns_error() {
        let err = parse_idl(&idl_path(), "DoesNotExist").unwrap_err();
        assert!(err.contains("DoesNotExist"), "error should name the account: {err}");
    }

    #[test]
    fn enum_type_returns_error_not_account() {
        // MarketState is an enum in types — must be rejected with a clear message
        let err = parse_idl(&idl_path(), "MarketState").unwrap_err();
        assert!(err.contains("enum"), "error should say it's an enum: {err}");
    }

    #[test]
    fn bad_path_returns_error() {
        let err = parse_idl(Path::new("/nonexistent/path/idl.json"), "EscrowVault").unwrap_err();
        assert!(err.contains("/nonexistent/path/idl.json"), "error should contain the missing path: {err}");
    }

    #[test]
    fn malformed_empty_array_type_returns_unknown() {
        let ty = parse_field_type(&serde_json::json!({
            "array": []
        }));

        assert!(matches!(ty, FieldType::Unknown(_)));
    }
}
