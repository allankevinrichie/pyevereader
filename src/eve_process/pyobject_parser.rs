use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::os::windows::prelude::OsStringExt;
use crate::eve_process::process::MemoryRegion;
use crate::eve_process::py_struct::*;
use crate::eve_process::pyobject_node::PyObjectNode;

impl PyObjectNode {
    pub fn parse_str(&self, regions: &HashMap<u64, MemoryRegion>) -> io::Result<String> {
        let region = regions.get(&self.base_addr).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "missing node region.")
        )?;
        let str_view = region.view_bytes_as::<CPyStringObject>(0, None)?;
        Ok(String::from_utf8_lossy(region.view_bytes(
            CPyStringObject::<1>::OFFSET_OB_SVAL.offset(),
            str_view.ob_base.ob_size as usize
        )?).to_string())
    }

    pub fn parse_unicode(&self, regions: &HashMap<u64, MemoryRegion>) -> io::Result<String> {
        let region = regions.get(&self.base_addr).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "missing node region.")
        )?;
        let unicode_view = region.view_bytes_as::<CPyUnicodeObject>(0, None)?;
        let str_len = unicode_view.length;
        let raw_wide_str: Vec<_> = regions.get(&unicode_view.str).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "missing unicode str region.")
        )?.view_bytes_as_vec_of::<u16>(0, (str_len as u64 * size_of::<u16>() as u64) as usize)?
            .iter().map(|&x| *x).collect();
        Ok(OsString::from_wide(&raw_wide_str).to_string_lossy().into_owned())
    }

    pub fn parse_int(&self, regions: &HashMap<u64, MemoryRegion>) -> io::Result<i64> {
        let region = regions.get(&self.base_addr).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "missing node region.")
        )?;
        let int_view = region.view_bytes_as::<CPyIntObject>(0, None)?;
        Ok(int_view.ob_ival as i64)
    }

    pub fn parse_float(&self, regions: &HashMap<u64, MemoryRegion>) -> io::Result<f64> {
        let region = regions.get(&self.base_addr).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "missing node region.")
        )?;
        let float_view = region.view_bytes_as::<CPyFloatObject>(0, None)?;
        Ok(float_view.ob_fval)
    }

    pub fn parse_bool(&self, regions: &HashMap<u64, MemoryRegion>) -> io::Result<bool> {
        let region = regions.get(&self.base_addr).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "missing node region.")
        )?;
        let bool_view = region.view_bytes_as::<CPyIntObject>(0, None)?;
        Ok(bool_view.ob_ival != 0)
    }

    pub fn parse_long(&self, regions: &HashMap<u64, MemoryRegion>) -> io::Result<i64> {
        let region = regions.get(&self.base_addr).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "missing node region.")
        )?;
        let long_view = region.view_bytes_as::<CPyLongObject>(0, None)?;
        let ob_size = long_view.ob_base.ob_size;
        Ok(region.view_bytes_as_vec_of::<u64>(
            CPyLongObject::<1>::OFFSET_OB_DIGIT.offset(),
            (ob_size.abs() as u64 * size_of::<u64>() as u64) as usize
        )?.into_iter().enumerate().map(
            |(i, d)| (*d as i64) * 2_i64.pow(30_u32 * i as u32)
        ).reduce(|acc, x| acc + x).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "parse_long failed")
        )? * (if ob_size < 0 {-1} else if ob_size > 0 {1} else { 0 }))
    }
}
