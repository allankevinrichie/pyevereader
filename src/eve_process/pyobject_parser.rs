use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt::Formatter;
use std::{io, slice};
use std::os::windows::prelude::OsStringExt;
use std::rc::Rc;
use libc::{abs, c_char};
use tracing_subscriber::reload::Handle;
use crate::eve_process::eve_process::{PyObjectNode, EVEProcess};
use crate::eve_process::py_struct::{CPyDictEntry, CPyDictObject, CPyFloatObject, CPyIntObject, CPyListObject, CPyLongObject, CPyObject, CPyStringObject, CPyTypeObject, CPyUnicodeObject};

impl EVEProcess {
    
     pub fn parse_node(&mut self, addr: u64) -> io::Result<()> {
         let node;
         if !self.objects.contains_key(&addr) {
             return Err(io::Error::new(
                 io::ErrorKind::InvalidInput,
                 format!("Node@(0x{:X}) doesn't exist.", addr)
             ))
         }
         node = self.objects.get(&addr).unwrap();
         match node.tp_name.as_str() { 
             "dict" => {
                 self.parse_dict(node.base_addr)
             },
             // "list" => {
             //     self.parse_list(node)
             // },
             // "str" => {
             //     self.parse_str(node)
             // },
             // "unicode" => {
             //     self.parse_unicode(node)
             // },
             // "NoneType" => {
             //     self.parse_NoneType(node)
             // },
             // "int" => {
             //     self.parse_int(node)
             // },
             // "float" => {
             //     self.parse_float(node)
             // },
             // "long" => {
             //     self.parse_long(node)
             // },
             // "bool" => {
             //     self.parse_bool(node)
             // },
             _ => {
                 todo!()
             }
         }
     }
     pub fn parse_dict(&mut self, addr: u64) -> io::Result<()> {
         let node = self.objects.get_mut(&addr).ok_or(
             io::Error::new(
                 io::ErrorKind::InvalidInput,
                 format!("Can't find object at 0x{:X} to parse", addr)
             )
         )?;
         if node.tp_name != "dict" {
             return Err(io::Error::new(
                 io::ErrorKind::InvalidInput,
                 format!("parse_dict expect a PyObjectNode of type `dict`, get `{}`", node.tp_name)
             ))
         }
         let attr_dict_view = self.regions.get_mut(&node.base_addr).ok_or(
             io::Error::new(
                 io::ErrorKind::InvalidInput,
                 format!("Can't find region at 0x{:X}", node.base_addr)
             )
         )?.view_bytes_as::<CPyDictObject>(0, None)?;
         let mask = attr_dict_view.ma_mask;
         let ma_table = attr_dict_view.ma_table;

         for i in 0..mask+1 {
             if let Ok(entry_region) = self.process.read_memory(
                     ma_table + (i as usize * size_of::<CPyDictEntry>()) as u64,
                     size_of::<CPyDictEntry>())
             {
                 if let Ok(entry_view) = entry_region.view_bytes_as::<CPyDictEntry>(0, None) {
                     let me_key_addr = entry_view.me_key;
                     let me_value_addr = entry_view.me_value;
                     if me_key_addr == 0 || me_value_addr == 0 {
                         continue
                     }
                     if let Ok(_) = self.new_node(me_key_addr) {
                         // if let Ok(_) = self.new_node(me_value_addr) {
                         //     node.attrs.insert(me_key_addr, me_value_addr);
                         // } else { 
                         //     let _ = self.del_node(me_key_addr);
                         // }
                     }
                 }
             }
         }
         node.is_parsed = true;
         Ok(())
     }

    pub fn parse_list<'l>(&mut self, node: &'l mut PyObjectNode) -> io::Result<&'l mut PyObjectNode> {
        if node.tp_name != "list" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_list expect a PyObjectNode of type `list`, get `{}`", node.tp_name)
            ))
        }
        let list_region = self.regions.get_mut(&node.base_addr).ok_or(
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Can't find region at 0x{:X}", node.base_addr)
            )
        )?;
        let list_view = list_region.view_bytes_as::<CPyListObject>(0, None)?;
        let ob_size = list_view.ob_base.ob_size;
        let obj_list = list_region.view_bytes_as_vec_of::<u64>(
            CPyListObject::<1>::OFFSET_OB_ITEM.offset(),
            ob_size as usize
        )?;
        // for &obj_addr in obj_list {
        //     if let Ok(_) = self.new_node(obj_addr) {
        //         node.items.push(obj_addr);
        //     }
        // }
        Ok(node)
    }

    pub fn parse_str(&self, node: &PyObjectNode) -> io::Result<String> {
        if node.tp_name != "str" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_str expect a PyObjectNode of type `str`, get `{}`", node.tp_name)
            ))
        }
        let str_view = node.region.view_bytes_as::<CPyStringObject>(0, None)?;
        let str_len = str_view.ob_base.ob_size;
        let raw_char_array = node.region.view_bytes(
            (str_view.ob_sval.as_ptr() as u64 - node.base_addr) as usize,
            (str_len as u64 * size_of::<c_char>() as u64) as usize
        )?;
        Ok(String::from_utf8_lossy(raw_char_array).to_string())
    }

    pub fn parse_unicode(&self, node: &PyObjectNode) -> io::Result<String> {
        if node.tp_name != "unicode" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_unicode expect a PyObjectNode of type `unicode`, get `{}`", node.tp_name)
            ))
        };
        let unicode_view = node.region.view_bytes_as::<CPyUnicodeObject>(0, None)?;
        let str_len = unicode_view.length;
        let raw_wchar_region = self.process.read_cache(unicode_view.str, (str_len as u64 * size_of::<u16>() as u64) as usize)?;
        let raw_wchar_vec_view = raw_wchar_region.view_bytes_as_vec_of::<u16>(0, (str_len as u64 * size_of::<u16>() as u64) as usize)?;
        let raw_wchar_vec_copy: Vec<_> = raw_wchar_vec_view.into_iter().map(|x| *x).collect();
        Ok(OsString::from_wide(raw_wchar_vec_copy.as_slice()).to_string_lossy().into_owned())
    }

    pub fn parse_NoneType(&self, node: &PyObjectNode) -> io::Result<()> {
        if node.tp_name != "NoneType" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_NoneType expect a PyObjectNode of type `NoneType`, get `{}`", node.tp_name)
            ))
        }
        Ok(())
    }

    pub fn parse_int(&self, node: &PyObjectNode) -> io::Result<i64> {
        if node.tp_name != "int" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_int expect a PyObjectNode of type `int`, get `{}`", node.tp_name)
            ))
        }
        let int_view = node.region.view_bytes_as::<CPyIntObject>(0, None)?;
        Ok(int_view.ob_ival as i64)
    }

    pub fn parse_float(&self, node: &PyObjectNode) -> io::Result<f64> {
        if node.tp_name != "float" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_float expect a PyObjectNode of type `float`, get `{}`", node.tp_name)
            ))
        }
        let float_view = node.region.view_bytes_as::<CPyFloatObject>(0, None)?;
        Ok(float_view.ob_fval)
    }

    pub fn parse_bool(&self, node: &PyObjectNode) -> io::Result<bool> {
        if node.tp_name != "bool" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_bool expect a PyObjectNode of type `bool`, get `{}`", node.tp_name)
            ))
        }
        let bool_view = node.region.view_bytes_as::<CPyIntObject>(0, None)?;
        Ok(bool_view.ob_ival != 0)
    }

    pub fn parse_long(&self, node: &PyObjectNode) -> io::Result<i64> {
        if node.tp_name != "long" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("parse_long expect a PyObjectNode of type `long`, get `{}`", node.tp_name)
            ))
        }
        let long_view = node.region.view_bytes_as::<CPyLongObject>(0, None)?;
        let ob_size = long_view.ob_base.ob_size;
        Ok(node.region.view_bytes_as_vec_of::<u64>(
            (long_view.ob_digit.as_ptr() as u64 - node.base_addr) as usize,
            (ob_size.abs() as u64 * size_of::<u64>() as u64) as usize
        )?.into_iter().enumerate().map(
            |(i, d)| (*d as i64) * 2_i64.pow(30_u32 * i as u32)
        ).reduce(|acc, x| acc + x).ok_or(
            io::Error::new(io::ErrorKind::InvalidInput, "parse_long failed")
        )? * (if ob_size < 0 {-1} else if ob_size > 0 {1} else { 0 }))
    }

 }
