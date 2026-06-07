


use std::{collections::HashMap};

use crate::{borsh::{FieldLayout, Layout}, idl::FieldType};

#[derive(Debug,Clone,PartialEq)]
pub enum ChangeKind{
    Unchanged,

    Added {at_index:usize},

    Removed {from_index:usize},

    TypeChanged{
        old_type : FieldType,
        new_type : FieldType
    },

    Reordered{
        old_index : usize,
        new_index : usize
    },

    TypeChangedAndReordered{
        old_type : FieldType,
        old_index: usize,
        new_type : FieldType,
        new_index: usize
    },
}

// diff engine Will produce Vec<FieldChange>
// classifier will consume Vec<FieldChange> 
#[derive(Debug,Clone,PartialEq)]
pub struct FieldChange{
    pub name : String,

    pub kind : ChangeKind,

    // field info for a old idl 
    // None for new Field 
    pub old_layout : Option<FieldLayout>,

    // field info for new idl
    // None for removed field
    pub new_layout : Option<FieldLayout>

}

pub fn diff(old:&Layout,new : &Layout) -> Vec<FieldChange>{
    let mut changes = Vec::new();

    let old_map:HashMap<&str,&FieldLayout>= old.fields.iter().map(|f|(f.name.as_str(),f)).collect(); 

    let new_map:HashMap<&str,&FieldLayout> = new.fields.iter().map(|f|(f.name.as_str(),f)).collect();

    for old_field in &old.fields{
        // check old fields in new map because if old filed is exist in new map then {Unchanged/TypeChanged/Reorderd/}

        if let Some(new_field) = new_map.get(old_field.name.as_str()){
            let kind = classify_change(old_field, new_field);

            changes.push(FieldChange{
                name : old_field.name.clone(),
                kind,
                old_layout : Some(old_field.clone()),
                new_layout : Some((*new_field).clone())
            });
        }else {
            // Field is Present in old Field it is removed
            changes.push(FieldChange{
                name : old_field.name.clone(),
                kind : ChangeKind::Removed { from_index: old_field.index },
                old_layout : Some(old_field.clone()),
                new_layout : None
            });
        }
    }

    // in this case if field is not present in old and just freshly added in new

    for new_field in &new.fields{
        // it checks for new filed in old map so this means field is added into new version
        if !old_map.contains_key(new_field.name.as_str()) {
            changes.push(FieldChange{
                name : new_field.name.clone(),
                kind : ChangeKind::Added { at_index: new_field.index },
                old_layout : None,
                new_layout : Some(new_field.clone())
            });
        }
    }

    // Sort by old index first, then new index
    // This gives a stable, readable order in the output:
    // existing fields in their original order, then added fields at the end
    changes.sort_by_key(|c| match &c.kind {
    ChangeKind::Removed { from_index } => *from_index,

    _ => c
        .new_layout
        .as_ref()
        .expect("non-removed changes always have new_layout")
        .index,
    });
    changes

}

fn classify_change(old:&FieldLayout,new : &FieldLayout)->ChangeKind{
    let typed_change = old.ty != new.ty;
    let index_change = old.index != new.index;

    match (typed_change,index_change){
        (false,false) => ChangeKind::Unchanged,

        (true,false) => ChangeKind::TypeChanged {
            old_type: old.ty.clone(),
            new_type: new.ty.clone()
        },

        (false,true) => ChangeKind::Reordered {
            old_index: old.index,
            new_index: new.index
        },

        (true,true) => ChangeKind::TypeChangedAndReordered { 
            old_type: old.ty.clone(),
            old_index: old.index,
            new_type: new.ty.clone(),
            new_index: new.index
        }

    }
}

// fn types_equal(a:&FieldType,b:&FieldType) -> bool{
//     match (a,b) {
//         (FieldType::U8,     FieldType::U8)     => true,
//         (FieldType::U16,    FieldType::U16)    => true,
//         (FieldType::U32,    FieldType::U32)    => true,
//         (FieldType::U64,    FieldType::U64)    => true,
//         (FieldType::U128,   FieldType::U128)   => true,
//         (FieldType::I8,     FieldType::I8)     => true,
//         (FieldType::I16,    FieldType::I16)    => true,
//         (FieldType::I32,    FieldType::I32)    => true,
//         (FieldType::I64,    FieldType::I64)    => true,
//         (FieldType::I128,   FieldType::I128)   => true,
//         (FieldType::Bool,   FieldType::Bool)   => true,
//         (FieldType::F32,    FieldType::F32)    => true,
//         (FieldType::F64,    FieldType::F64)    => true,
//         (FieldType::Pubkey, FieldType::Pubkey) => true,
//         (FieldType::String, FieldType::String) => true,

//         (FieldType::Vec(a_inner),    FieldType::Vec(b_inner))    => types_equal(a_inner, b_inner),
//         (FieldType::Option(a_inner), FieldType::Option(b_inner)) => types_equal(a_inner, b_inner),
//         (FieldType::Array(a_inner, a_len), FieldType::Array(b_inner, b_len)) => {
//             a_len == b_len && types_equal(a_inner, b_inner)
//         }

//         (FieldType::Unknown(a_s), FieldType::Unknown(b_s)) => a_s == b_s,

//         (FieldType::Defined(a_name), FieldType::Defined(b_name)) => {
//           a_name == b_name
//         },

//         // Anything else — different types
//         _ => false,
//     }
// }


#[cfg(test)]
mod tests {
    use super::*;
    use crate::idl::{AccountDef, FieldDef, FieldType};
    use crate::borsh::{compute_layout};

    fn make_layout(name: &str, fields: Vec<(&str, FieldType)>) -> Layout {
        let def = AccountDef {
            name: name.to_string(),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(i, (n, ty))| FieldDef {
                    name: n.to_string(),
                    ty,
                    index: i,
                })
                .collect(),
        };
        compute_layout(&def)
    }

    // ── Test 1: nothing changed ──
    #[test]
    fn unchanged() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
        ]);
        let changes = diff(&old, &new);
        assert!(changes.iter().all(|c| c.kind == ChangeKind::Unchanged));
    }

    // ── Test 2: field added at end ──
    #[test]
    fn field_added_at_end() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
            ("bump",      FieldType::U8),   // new
        ]);
        let changes = diff(&old, &new);
        let added: Vec<_> = changes.iter()
            .filter(|c| matches!(c.kind, ChangeKind::Added { .. }))
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].name, "bump");
    }

    // ── Test 3: field removed ──
    #[test]
    fn field_removed() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
            ("bump",      FieldType::U8),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),
            // bump removed
        ]);
        let changes = diff(&old, &new);
        let removed: Vec<_> = changes.iter()
            .filter(|c| matches!(c.kind, ChangeKind::Removed { .. }))
            .collect();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name, "bump");
    }

    // ── Test 4: type changed ──
    #[test]
    fn type_changed() {
        let old = make_layout("Vault", vec![
            ("balance", FieldType::U32),  // was u32
        ]);
        let new = make_layout("Vault", vec![
            ("balance", FieldType::U64),  // now u64
        ]);
        let changes = diff(&old, &new);
        assert!(matches!(
            changes[0].kind,
            ChangeKind::TypeChanged { .. }
        ));
    }

    // ── Test 5: field reordered ──
    #[test]
    fn field_reordered() {
        let old = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("bump",      FieldType::U8),
            ("balance",   FieldType::U64),
        ]);
        let new = make_layout("Vault", vec![
            ("authority", FieldType::Pubkey),
            ("balance",   FieldType::U64),  // moved up
            ("bump",      FieldType::U8),
        ]);
        let changes = diff(&old, &new);
        let reordered: Vec<_> = changes.iter()
            .filter(|c| matches!(c.kind, ChangeKind::Reordered { .. }))
            .collect();
        assert!(!reordered.is_empty());
    }
}