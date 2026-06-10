use crate::{diff::{ChangeKind, FieldChange}, idl::FieldType};

#[derive(Debug,Clone,PartialEq)]
pub enum  Safety {
    Safe,
    Review,
    Danger
}

#[derive(Debug,Clone)]
pub struct ClassifiedChanges{
    pub changes : FieldChange,

    pub safety : Safety,

    pub reason : &'static str,
}


pub fn classify_one(changes : FieldChange)-> ClassifiedChanges{
    
    let (safety,reason) = match &changes.kind {
        
        // Unchanged
        ChangeKind::Unchanged =>(
            Safety::Safe, 
            "Field Unchanged"
        ),

        ChangeKind::Added { .. } => (
            Safety::Safe,
            "Field Added"
        ),

        ChangeKind::Removed { .. } => (
            Safety::Danger,
            "field removed — permanent data loss, suggest marking deprecated instead"
        ),

        ChangeKind::Reordered { ..} => (
            Safety::Safe,
            "field reordered — safe for borsh, serialization matches by name"
        )
        ,
        ChangeKind::TypeChanged { old_type,new_type } =>{ 
            classify_type_change(old_type, new_type)
        }

         ChangeKind::TypeChangedAndReordered { old_type,new_type, .. } => {
            classify_type_change(old_type, new_type)
        }


    };
    
    ClassifiedChanges { changes, safety , reason  }
}


fn classify_type_change(old_ty:&FieldType,new_ty: &FieldType)->(Safety, &'static str){
    if is_safe_widen(&old_ty, &new_ty){
        return (
            Safety::Safe,
            "unsigned/signed integer widen — value fits but verify no signedness assumption",
        );
    }

    if is_narrowing(&old_ty, &new_ty){
        return (
            Safety::Danger,
             "narrowing type change — possible overflow or data truncation",
        );
    }


    if is_sign_flip(old_ty, new_ty){
        return (
        Safety::Danger,
        "sign flip — same bytes reinterpreted, values will be wrong for large numbers"
        );
    }


    if is_float_int_changes(old_ty, new_ty) {
        return (
            Safety::Danger,
            "float/integer reinterpretation — bits mean completely different things",
        );
    }

        // String ↔ anything
    // Risk: variable length encoding vs fixed, completely incompatible
    if matches!(old_ty, FieldType::String) || matches!(new_ty, FieldType::String) {
        return (
            Safety::Danger,
            "string reinterpretation — variable length encoding incompatible with fixed types",
        );
    }

    // Vec ↔ non-Vec
    if matches!(old_ty, FieldType::Vec(_)) != matches!(new_ty, FieldType::Vec(_)) {
        return (
            Safety::Danger,
            "vec reinterpretation — length-prefixed encoding incompatible with other types",
        );
    }

    // Unknown type involved — can't reason about it
    if matches!(old_ty, FieldType::Unknown(_)) || matches!(new_ty, FieldType::Unknown(_)) {
        return (
            Safety::Danger,
            "unknown type involved — cannot reason about byte-level safety",
        );
    }

    (
        Safety::Danger,
        "type reinterpretation — bytes now mean something different"
    )
}


fn is_safe_widen(old_ty:&FieldType,new_ty:&FieldType)->bool{
    matches!(
        (old_ty,new_ty),

        (FieldType::U8,FieldType::U16)    |
        (FieldType::U8,  FieldType::U32)  |
        (FieldType::U8,  FieldType::U64)  |
        (FieldType::U8,  FieldType::U128) |
        (FieldType::U16, FieldType::U32)  |
        (FieldType::U16, FieldType::U64)  |
        (FieldType::U16, FieldType::U128) |
        (FieldType::U32, FieldType::U64)  |
        (FieldType::U32, FieldType::U128) |
        (FieldType::U64, FieldType::U128) |


        (FieldType::I8,  FieldType::I16)  |
        (FieldType::I8,  FieldType::I32)  |
        (FieldType::I8,  FieldType::I64)  |
        (FieldType::I8,  FieldType::I128) |
        (FieldType::I16, FieldType::I32)  |
        (FieldType::I16, FieldType::I64)  |
        (FieldType::I16, FieldType::I128) |
        (FieldType::I32, FieldType::I64)  |
        (FieldType::I32, FieldType::I128) |
        (FieldType::I64, FieldType::I128) | 
        (FieldType::F32, FieldType::F64)
    )
}


fn is_narrowing(old_ty: &FieldType, new_ty: &FieldType) -> bool {
    matches!(
        (old_ty, new_ty),
        // Unsigned narrowing
        (FieldType::U128, FieldType::U64)  |
        (FieldType::U128, FieldType::U32)  |
        (FieldType::U128, FieldType::U16)  |
        (FieldType::U128, FieldType::U8)   |
        (FieldType::U64,  FieldType::U32)  |
        (FieldType::U64,  FieldType::U16)  |
        (FieldType::U64,  FieldType::U8)   |
        (FieldType::U32,  FieldType::U16)  |
        (FieldType::U32,  FieldType::U8)   |
        (FieldType::U16,  FieldType::U8)   |

        // Signed narrowing
        (FieldType::I128, FieldType::I64)  |
        (FieldType::I128, FieldType::I32)  |
        (FieldType::I128, FieldType::I16)  |
        (FieldType::I128, FieldType::I8)   |
        (FieldType::I64,  FieldType::I32)  |
        (FieldType::I64,  FieldType::I16)  |
        (FieldType::I64,  FieldType::I8)   |
        (FieldType::I32,  FieldType::I16)  |
        (FieldType::I32,  FieldType::I8)   |
        (FieldType::I16,  FieldType::I8)   |

        // Float narrowing
        (FieldType::F64,  FieldType::F32)
    )
}

// ─────────────────────────────────────────────
// is_sign_flip — same width, different signedness
// ─────────────────────────────────────────────
fn is_sign_flip(old_ty: &FieldType, new_ty: &FieldType) -> bool {
    matches!(
        (old_ty, new_ty),
        (FieldType::U8,   FieldType::I8)   |
        (FieldType::I8,   FieldType::U8)   |
        (FieldType::U16,  FieldType::I16)  |
        (FieldType::I16,  FieldType::U16)  |
        (FieldType::U32,  FieldType::I32)  |
        (FieldType::I32,  FieldType::U32)  |
        (FieldType::U64,  FieldType::I64)  |
        (FieldType::I64,  FieldType::U64)  |
        (FieldType::U128, FieldType::I128) |
        (FieldType::I128, FieldType::U128)
    )
}

fn is_float_int_changes(old_ty:&FieldType,new_ty:&FieldType)->bool{
    let old_is_float = matches!(old_ty,FieldType::F32 | FieldType::F64);
    let new_is_float = matches!(new_ty, FieldType::F32 | FieldType::F64);

        let old_is_int = matches!(
        old_ty,
        FieldType::U8  | FieldType::U16 | FieldType::U32 |
        FieldType::U64 | FieldType::U128 |
        FieldType::I8  | FieldType::I16 | FieldType::I32 |
        FieldType::I64 | FieldType::I128
    );
    let new_is_int = matches!(
        new_ty,
        FieldType::U8  | FieldType::U16 | FieldType::U32 |
        FieldType::U64 | FieldType::U128 |
        FieldType::I8  | FieldType::I16 | FieldType::I32 |
        FieldType::I64 | FieldType::I128
    );

     // True if one side is float and the other is integer
    (old_is_float && new_is_int) || (old_is_int && new_is_float)
}