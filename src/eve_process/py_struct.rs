use std::io;
use std::io::Error;
use std::mem::ManuallyDrop;
use lazy_static::lazy_static;
use libc::*;
use loop_code::repeat;


macro_rules! rpointer {
    ($T:ty) => {u64};
    () => {u64}
}

type rpyobject = rpointer![CPyObject];

macro_rules! rarray {
    ($T:ty, $n:expr) => {[$T; $n]};
    ($T:ty) => {[$T; 1]};
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyObject {
    pub ob_refcnt: ssize_t,
    pub ob_type: rpointer![CPyTypeObject],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyVarObject {
    pub ob_refcnt: ssize_t,
    pub ob_type: rpointer![CPyTypeObject],
    pub ob_size: ssize_t,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyTypeObject {
    pub ob_base: CPyVarObject,
    pub tp_name: rpointer![c_char]
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyByteArrayObject<const N: usize = 1> {
    pub ob_base: CPyVarObject,
    pub ob_exports: c_int,
    pub ob_alloc: ssize_t,
    pub ob_bytes: rpointer![c_char]
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyStringObject<const N: usize = 1> {
    pub ob_base: CPyVarObject,
    pub ob_shash: c_long,
    pub ob_sstate: c_int,
    pub ob_sval: rarray![c_char, N]
}

pub type CPyBytesObject<const N: usize = 1> = CPyStringObject<N>;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyDictEntry {
    pub me_hash: ssize_t,
    pub me_key: rpyobject,
    pub me_value: rpyobject
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyDictObject {
    pub ob_base: CPyObject,
    pub ma_fill: ssize_t,
    pub ma_used: ssize_t,
    pub ma_mask: ssize_t,
    pub ma_table: rpointer![CPyDictEntry],
    pub ma_lookup: rpointer![],
    pub ma_smalltable: rarray![CPyDictEntry, 8]
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyFloatObject {
    pub ob_base: CPyObject,
    pub ob_fval: c_double
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyIntObject {
    pub ob_base: CPyObject,
    pub ob_ival: c_long
}

pub type CPyBoolObject = CPyIntObject;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyListObject<const N: usize = 1> {
    pub ob_base: CPyVarObject,
    pub ob_item: rarray![rpyobject, N],
    pub allocated: ssize_t
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyLongObject<const N: usize = 1> {
    pub ob_base: CPyVarObject,
    pub ob_digit: rarray![u32, N]
}

#[repr(C)]
pub struct CPySetEntry {
    pub hash: c_long,
    pub key: rpyobject
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPySetObject {
    pub ob_base: CPyObject,
    pub fill: ssize_t,
    pub used: ssize_t,
    pub mask: ssize_t,
    pub table: rpointer![CPySetEntry]
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyTupleObject<const N: usize = 1> {
    pub ob_base: CPyVarObject,
    pub ob_item: rarray![rpyobject, N]
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyUnicodeObject {
    pub ob_base: CPyObject,
    pub length: ssize_t,
    pub str: rpointer![wchar_t],
    pub hash: c_long,
    pub defenc: rpyobject
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CPyCustomObject {
    pub ob_base: CPyObject,
    pub attributes: rpointer![CPyDictObject]
}
