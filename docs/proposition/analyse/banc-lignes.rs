//! Étude de layout uniquement : aucune modification des types du moteur.
use std::mem::{align_of, offset_of, size_of};
#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
struct Ligne64 {
    payload: u64,
    observed_ms: u64,
    received_ms: u64,
    changed_ms: u64,
    parent: u32,
    identity: u32,
    schema: u32,
    aux: u32,
    revision: u32,
    unit: u16,
    semantic: u16,
    flags: u16,
    scale: i8,
    decimals: u8,
    row_kind: u8,
    value_kind: u8,
    codec: u16,
}
// Même contenu, alignement naturel : 64 o ; enveloppe inline 96 o sans padding caché.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Base64 {
    payload: u64, observed_ms: u64, received_ms: u64, changed_ms: u64,
    parent: u32, identity: u32, schema: u32, aux: u32, revision: u32,
    unit: u16, semantic: u16, flags: u16,
    scale: i8, decimals: u8, row_kind: u8, value_kind: u8, codec: u16,
}
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Numeric32 { min: u64, max: u64, step: u64, anchor: u64 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Ligne96 { base: Base64, numeric: Numeric32 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Percent32 { low: u64, high: u64, version: u32, curve_ref: u32, mode: u16, reserved: [u8;6] }
#[repr(C, align(64))]
struct Ligne96Alignee64 { base: Base64, numeric: Numeric32 }
fn main() {
    assert_eq!(size_of::<Ligne64>(),64);
    assert_eq!(align_of::<Ligne64>(),64);
    assert_eq!(size_of::<Base64>(),64);
    assert_eq!(size_of::<Numeric32>(),32);
    assert_eq!(size_of::<Percent32>(),32);
    assert_eq!(size_of::<Ligne96>(),96);
    assert_eq!(align_of::<Ligne96>(),8);
    assert_eq!(size_of::<Ligne96Alignee64>(),128);
    assert_eq!(offset_of!(Ligne64, codec),62);
    let count=std::env::args().nth(1).map_or(2760, |n| n.parse::<usize>().expect("nombre de lignes"));
    assert!(count >= 2);
    let rows = vec![Ligne64::default();count];
    assert_eq!((rows.as_ptr() as usize)%64,0);
    assert_eq!((&rows[1] as *const _ as usize)-(rows.as_ptr() as usize),64);
    let rows96=vec![Ligne96::default();count];
    assert_eq!((&rows96[1] as *const _ as usize)-(rows96.as_ptr() as usize),96);
    println!("{{\"target\":\"linux-{}\",\"line64\":{},\"alignment64\":{},\"line96\":{},\"alignment96\":{},\"numeric32\":{},\"percent32\":{},\"verified_rows\":{},\"allocated_rows64_bytes\":{},\"allocated_rows96_bytes\":{}}}",std::env::consts::ARCH,size_of::<Ligne64>(),align_of::<Ligne64>(),size_of::<Ligne96>(),align_of::<Ligne96>(),size_of::<Numeric32>(),size_of::<Percent32>(),count,rows.len()*size_of::<Ligne64>(),rows96.len()*size_of::<Ligne96>());
}
