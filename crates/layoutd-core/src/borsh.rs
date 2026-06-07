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

#[derive(Debug,Clone,PartialEq)]
pub struct FieldLayout{
    pub  name : String,
    pub  ty : FieldType,
    pub  index : usize,
    pub  offset : Offset,
    pub  size : Size
}

#[derive(Debug,Clone,PartialEq)]
pub struct Layout{
    pub account_name : String,
    pub fields  : Vec<FieldLayout>,
    pub total_size : Size
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

        // Option<T> = 1-byte flag for None, or 1-byte flag + T for Some(T)
        // The serialized size depends on the runtime value.
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

        fields.push(FieldLayout { name: field.name.clone(), ty: field.ty.clone(), index: field.index, offset , size });
    }

    let total_size = if hit_variable {
        Size::Variable
    } else {
        Size::Fixed(current_offset)
    };

    Layout { account_name: def.name.clone(), fields, total_size }
}


// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::idl::FieldDef;

//     fn account(fields: Vec<FieldDef>) -> AccountDef {
//         AccountDef {
//             name: "TestAccount".to_string(),
//             fields,
//         }
//     }

//     fn field(index: usize, name: &str, ty: FieldType) -> FieldDef {
//         FieldDef {
//             name: name.to_string(),
//             ty,
//             index,
//         }
//     }

//     #[test]
//     fn fixed_layout_preserves_indices_and_total_size() {
//         let def = account(vec![
//             field(0, "amount", FieldType::U64),
//             field(1, "owner", FieldType::Pubkey),
//             field(2, "bump", FieldType::U8),
//         ]);

//         let layout = compute_layout(&def);

//         assert_eq!(layout.total_size, Size::Fixed(49));
//         assert_eq!(layout.fields[0].index, 0);
//         assert_eq!(layout.fields[0].offset, Offset::Fixed(8));
//         assert_eq!(layout.fields[1].index, 1);
//         assert_eq!(layout.fields[1].offset, Offset::Fixed(16));
//         assert_eq!(layout.fields[2].index, 2);
//         assert_eq!(layout.fields[2].offset, Offset::Fixed(48));
//     }

//     #[test]
//     fn variable_field_makes_total_size_and_later_offsets_unknown() {
//         let def = account(vec![
//             field(0, "question", FieldType::String),
//             field(1, "creator", FieldType::Pubkey),
//         ]);

//         let layout = compute_layout(&def);

//         assert_eq!(layout.total_size, Size::Variable);
//         assert_eq!(layout.fields[0].offset, Offset::Fixed(8));
//         assert_eq!(layout.fields[1].index, 1);
//         assert_eq!(layout.fields[1].offset, Offset::AfterVariable);
//     }
// }

