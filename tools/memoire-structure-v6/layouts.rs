//! Candidate memory shapes only; these C layouts are not a persistence ABI.
#![allow(dead_code)]
use std::mem::{align_of, offset_of, size_of};

macro_rules! shape {
    ($name:ident {$($field:ident : $ty:ty),* $(,)?}) => {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub struct $name {$(pub $field: $ty),*}
    };
}
shape!(Row64 {
    id: u64,
    parent: u64,
    payload: u64,
    previous_payload: u64,
    received_ms: u64,
    changed_ms: u64,
    schema: u32,
    aux: u32,
    revision: u32,
    flags: u16,
    row_kind: u8,
    value_kind: u8
});
shape!(Numeric32 {
    min: u64,
    max: u64,
    step: u64,
    anchor: u64
});
shape!(Row96 {
    base: Row64,
    numeric: Numeric32
});
shape!(Hot32 {
    payload: u64,
    previous_payload: u64,
    received_ms: u64,
    changed_ms: u64
});
shape!(Cold32 {
    id: u64,
    parent: u64,
    schema: u32,
    aux: u32,
    revision: u32,
    flags: u16,
    row_kind: u8,
    value_kind: u8
});
shape!(Alias32 {
    alias_id: u64,
    owner_id: u64,
    source_id: u64,
    source_slot: u32,
    source_generation: u32
});
shape!(GlobalSlot16 {
    physical_index: u32,
    generation: u32,
    meta_index: u32,
    arena: u8,
    flags: u8,
    reserved: u16
});
shape!(Handle8 {
    slot: u32,
    generation: u32
});
shape!(Meta32 {
    label: u32,
    native_key: u32,
    location: u32,
    policy: u32,
    read_plan: u32,
    write_plan: u32,
    executable: u32,
    extra: u32
});
shape!(Schema64 {
    unit: u32,
    semantic: u16,
    numeric_repr: u8,
    decimals: i8,
    prefix_exp10: i8,
    value_kind: u8,
    flags: u16,
    max_bytes: u32,
    max_items: u32,
    composite_schema: u32,
    native_min: u64,
    native_max: u64,
    scale_num: i64,
    scale_den: i64,
    reserved: u64
});
shape!(Transform64 {
    mul: f64,
    add: f64,
    in_min: f64,
    in_max: f64,
    out_min: f64,
    out_max: f64,
    revision: u32,
    opcode: u16,
    flags: u16,
    input_kind: u8,
    output_kind: u8,
    rounding: u8,
    overflow: u8,
    reserved: u32
});
shape!(Edge24 {
    from: u64,
    to: u64,
    relation: u16,
    flags: u16,
    revision: u32
});
shape!(Target12 {
    to: u32,
    relation: u16,
    flags: u16,
    revision: u32
});
shape!(PoolEntry16 {
    offset: u32,
    len: u32,
    generation: u32,
    kind: u16,
    flags: u16
});
shape!(Slice8 {
    offset: u32,
    len: u32
});
shape!(Component32 {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    e: u32,
    f: u32,
    g: u32,
    h: u32
});
shape!(Calc48 {
    definition_id: u64,
    deadline_tick: u64,
    source_ts: u64,
    version: u32,
    first_edge: u32,
    edge_count: u16,
    operation: u8,
    flags: u8,
    history_ref: u32,
    unit_ref: u32,
    reserved: u32
});
shape!(Route32 {
    point_id: u64,
    handle: Handle8,
    read_plan: u32,
    write_plan: u32,
    native_key: u32,
    flags: u16,
    codec: u8,
    access: u8
});

pub fn json() -> String {
    let mut entries = Vec::new();
    macro_rules! layout {
        ($ty:ty; $($field:ident),+ $(,)?) => {{
            let fields = vec![$(format!("\"{}\":{}", stringify!($field), offset_of!($ty, $field))),+];
            entries.push(format!("{{\"name\":\"{}\",\"size\":{},\"align\":{},\"offsets\":{{{}}}}}",
                stringify!($ty), size_of::<$ty>(), align_of::<$ty>(), fields.join(",")));
        }};
    }
    layout!(Row64; id,parent,payload,previous_payload,received_ms,changed_ms,schema,aux,revision,flags,row_kind,value_kind);
    layout!(Numeric32; min,max,step,anchor);
    layout!(Row96; base,numeric);
    layout!(Hot32; payload,previous_payload,received_ms,changed_ms);
    layout!(Cold32; id,parent,schema,aux,revision,flags,row_kind,value_kind);
    layout!(Alias32; alias_id,owner_id,source_id,source_slot,source_generation);
    layout!(GlobalSlot16; physical_index,generation,meta_index,arena,flags,reserved);
    layout!(Handle8; slot,generation);
    layout!(Meta32; label,native_key,location,policy,read_plan,write_plan,executable,extra);
    layout!(Schema64; unit,semantic,numeric_repr,decimals,prefix_exp10,value_kind,flags,max_bytes,max_items,composite_schema,native_min,native_max,scale_num,scale_den,reserved);
    layout!(Transform64; mul,add,in_min,in_max,out_min,out_max,revision,opcode,flags,input_kind,output_kind,rounding,overflow,reserved);
    layout!(Edge24; from,to,relation,flags,revision);
    layout!(Target12; to,relation,flags,revision);
    layout!(PoolEntry16; offset,len,generation,kind,flags);
    layout!(Slice8; offset,len);
    layout!(Component32; a,b,c,d,e,f,g,h);
    layout!(Calc48; definition_id,deadline_tick,source_ts,version,first_edge,edge_count,operation,flags,history_ref,unit_ref,reserved);
    layout!(Route32; point_id,handle,read_plan,write_plan,native_key,flags,codec,access);
    format!(
        "{{\"pointer_bytes\":{},\"endianness\":\"{}\",\"structures\":[{}]}}",
        size_of::<usize>(),
        if cfg!(target_endian = "little") {
            "little"
        } else {
            "big"
        },
        entries.join(",")
    )
}
