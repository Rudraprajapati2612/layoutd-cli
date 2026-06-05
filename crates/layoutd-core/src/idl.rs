pub struct  AccountDef{
    pub name : String,
    pub filed : Vec<FieldDef>
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