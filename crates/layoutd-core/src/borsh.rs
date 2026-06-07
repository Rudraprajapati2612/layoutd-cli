use crate::idl::{AccountDef, FieldType};


#[derive(Debug,Clone,PartialEq)]
pub enum Size{
    Fixed(usize),
    Variable
}

#[derive(Debug,Clone,PartialEq)]
pub enum Offset {
    Fixed(usize),
    AfterVariable
}

#[derive(Debug,Clone)]
pub struct FieldLayout{
    pub  name : String,
    pub  ty : FieldType,
    pub  offset : Offset,
    pub  size : Size
}

#[derive(Debug,Clone)]
pub struct Layout{
    pub account_name : String,
    pub fields  : Vec<FieldLayout>
}

//  it find the size of all the Field types  
pub fn size_of(ty: &FieldType) -> Size{
    match ty {
        FieldType::U8 => Size::Fixed(1),
        FieldType::I8 => Size::Fixed(1),
        FieldType::Bool => Size::Fixed(1),

        // 2 byte types
        FieldType::U16  => Size::Fixed(2),
        FieldType::I16  => Size::Fixed(2),

        // 4 bytes types
         FieldType::U32  => Size::Fixed(4),
        FieldType::I32  => Size::Fixed(4),
        FieldType::F32  => Size::Fixed(4),

        // 8-byte types
        FieldType::U64  => Size::Fixed(8),
        FieldType::I64  => Size::Fixed(8),
        FieldType::F64  => Size::Fixed(8),

        // 16-byte types
        FieldType::U128 => Size::Fixed(16),
        FieldType::I128 => Size::Fixed(16),

        // Pubkey is fixed and size is 32 bytes

        FieldType::Pubkey => Size::Fixed(32),

        // for string size is variable 
        FieldType::String => Size::Variable,

        FieldType::Vec(_) => Size::Variable,


        // Array<T, N> = N × size_of(T)
        // Only Fixed if inner type is Fixed
        // Example: [u8; 32] = Fixed(32), [String; 4] = Variable
        FieldType::Array(inner, n) => {
            match size_of(inner) {
                Size::Fixed(inner_size) => Size::Fixed(inner_size * n),
                Size::Variable => Size::Variable,
            }
        }

        // Option<T> = 1-byte flag + maybe T
        // If T is fixed → still Variable because the flag changes total size
        // Conservative: always Variable
        FieldType::Option(_) => Size::Variable,

        FieldType::Defined(_) => Size::Variable,
        // Unknown type — we can't know → Variable (conservative)
        // This handles `defined` types like MarketState, ResultOutcome
        // from your IDL that we can't see inside
        FieldType::Unknown(_) => Size::Variable,
        
    }
}


pub fn compute_layout(def:&AccountDef) -> Layout{

    // because always descriminator occupies 8 bytes  
    let mut current_offset:usize = 8;

    let mut hit_variable = false;

    let mut fields = Vec::new();

    for field in &def.fields{
        let size = size_of(&field.ty);

        let offset = if hit_variable{
            Offset::AfterVariable
        }else {
            Offset::Fixed(current_offset)
        };

        match &size{
            Size::Fixed(n)=>{
                if !hit_variable{
                    current_offset += n
                }
            }
            Size::Variable =>{

                hit_variable = true;
            }
        }

        fields.push(FieldLayout { name: field.name.clone(), ty: field.ty.clone(), offset , size });
    }

    Layout { account_name: def.name.clone(), fields }
}


