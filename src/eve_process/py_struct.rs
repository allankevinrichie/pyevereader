use std::mem::ManuallyDrop;
use libc::*;

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
pub struct CPyObject {
    pub ob_refcnt: ssize_t,
    pub ob_type: rpointer![CPyTypeObject],
}

#[repr(C)]
pub struct CPyVarObject {
    pub ob_refcnt: ssize_t,
    pub ob_type: rpointer![CPyTypeObject],
    pub ob_size: ssize_t,
}

#[repr(C)]
pub struct CPyTypeObject {
    pub ob_base: CPyVarObject,
    pub tp_name: rpointer![c_char]
}

#[repr(C)]
pub struct CPyByteArrayExtra {
    pub ob_base: CPyVarObject,
    pub ob_exports: c_int,
    pub ob_alloc: ssize_t,
    pub ob_bytes: rpointer![c_char]
}

#[repr(C)]
pub struct CPyStringObject {
    pub ob_base: CPyVarObject,
    pub ob_shash: c_long,
    pub ob_sstate: c_int,
    pub ob_sval: rarray![c_char]
}

type CPyBytesObject = CPyStringObject;

#[repr(C)]
pub struct CPyDictEntry {
    pub me_hash: ssize_t,
    pub me_key: rpyobject,
    pub me_value: rpyobject
}

#[repr(C)]
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
pub struct CPyFloatObject {
    pub ob_base: CPyObject,
    pub ob_fval: c_double
}

#[repr(C)]
pub struct CPyIntObject {
    pub ob_base: CPyObject,
    pub ob_ival: c_long
}

type CPyBoolObject = CPyIntObject;

#[repr(C)]
pub struct CPyListObject {
    pub ob_base: CPyVarObject,
    pub ob_item: rarray![rpyobject],
    pub allocated: ssize_t
}

#[repr(C)]
pub struct CPyLongObject {
    pub ob_base: CPyVarObject,
    pub ob_digit: rarray![u32]
}

#[repr(C)]
pub struct CPySetEntry {
    pub hash: c_long,
    pub key: rpyobject
}

#[repr(C)]
pub struct CPySetObject {
    pub ob_base: CPyObject,
    pub fill: ssize_t,
    pub used: ssize_t,
    pub mask: ssize_t,
    pub table: rpointer![CPySetEntry]
}

#[repr(C)]
pub struct CPyTupleObject {
    pub ob_base: CPyVarObject,
    pub ob_item: rarray![rpyobject]
}

#[repr(C)]
pub struct CPyUnicodeObject {
    pub ob_base: CPyObject,
    pub length: ssize_t,
    pub str: rpointer![wchar_t],
    pub hash: c_long,
    pub defenc: rpyobject
}

#[repr(C)]
pub struct CPyCustomObject {
    pub ob_base: CPyObject,
    pub attributes: rpointer![CPyDictObject]
}
