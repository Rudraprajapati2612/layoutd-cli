use serde::{Deserialize, Serialize};
use std::{ path::Path};


pub struct  AccountDef{
    pub name : String,
    pub fields : Vec<FieldDef>
}

pub struct FieldDef{
    pub name : String,
    pub ty : FieldType,
    pub index : usize
}

pub enum  FieldType {
    U8, U16, U32, U64, U128,
    I8, I16, I32, I64, I128,
    Bool,F32,F64,
    Pubkey,
    String,
    Vec(Box<FieldType>),
    Array(Box<FieldType>, usize),
    Option(Box<FieldType>),
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
    let content = std::fs::read_to_string(path).map_err(|e| format!("failed to Read Idl: {e}"))?;

    // deserialize(Converting a json formate into a rust struct) a raw idl into 

    let raw:RawIdl = serde_json::from_str(&content).map_err(|e| format!("Failed to parse Idl: {e}"))?;

    // Find the accountes by types  

    let accounts = raw.types.into_iter().find(|a| a.name == account_name).ok_or_else(|| format!("Account '{}' not found in IDL", account_name))?;

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
                let inner_ty = parse_field_type(&arr[0]);
                let len = arr[1].as_u64().unwrap_or(0) as usize;
                FieldType::Array(Box::new(inner_ty), len)
            } else if let Some(inner) = map.get("option") {
                FieldType::Option(Box::new(parse_field_type(inner)))
            } else {
                FieldType::Unknown(format!("{:?}", map))
            }
        },
        other => FieldType::Unknown(format!("{:?}", other)),
    
    }
}