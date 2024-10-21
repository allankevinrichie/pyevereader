use std::collections::HashMap;
use std::io;
use crate::eve_process::process::{MemoryRegion, Process};
use crate::eve_process::py_struct::*;

#[derive(Debug, Default)]
pub struct PyObjectNode {
    pub base_addr: u64,
    pub ob_type: u64,
    pub tp_name: String,
    pub attrs: HashMap<u64, u64>,
    pub items: Vec<u64>,
    pub extras: Vec<u64>,
    pub is_type: bool,
    pub is_resolved: bool
}

pub fn new_node(
    base_addr: u64,
    process: &Process,
    objects: &mut HashMap<u64, PyObjectNode>,
    regions: &mut HashMap<u64, MemoryRegion>
) -> io::Result<Vec<u64>>
{
    let children = Vec::new();
    if base_addr == 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "invalid base_addr"));
    }
    let mut tp_name_size = 255;
    let mut pyobj_region = process.read_memory(base_addr, size_of::<CPyObject>())?;
    let pyobj_view = pyobj_region.view_bytes_as::<CPyObject>(0, None)?;
    let pyobj_type_addr = pyobj_view.ob_type;

    let tp_name_inferred;
    // get existing type objects or put new one into cache, we assume that no new type object
    // will be created dynamically.
    if pyobj_type_addr == base_addr {
        if objects.contains_key(&base_addr) {
            return Ok(children);
        }
        let pyobj_type_region = process.read_memory(pyobj_type_addr, size_of::<CPyTypeObject>())?;
        let obj = PyObjectNode {
            base_addr,
            ob_type: pyobj_type_addr,
            tp_name: "type".to_string(),
            attrs: Default::default(),
            items: vec![],
            extras: vec![],
            is_type: true,
            is_resolved: true,
        };
        objects.insert(base_addr, obj);
        regions.insert(base_addr, pyobj_type_region);
        return Ok(children);
    }  else {
        if !objects.contains_key(&pyobj_type_addr) {
            new_node(pyobj_type_addr, process, objects, regions)?;
        }
        let tp_obj = objects.get(&pyobj_type_addr).ok_or(
            io::Error::new(io::ErrorKind::Other, "invalid ob_type")
        )?;
        tp_name_inferred = tp_obj.tp_name.clone();
        if tp_name_inferred == "type" {
            let pyobj_type_region = process.read_memory(base_addr, size_of::<CPyTypeObject>())?;
            let pyobj_tp_name_addr = pyobj_type_region.view_bytes_as::<CPyTypeObject>(0, None)?.tp_name;
            let pyobj_tp_name_region = process.read_cache(pyobj_tp_name_addr, tp_name_size)?;
            let pyobj_tp_name = &pyobj_tp_name_region.data;
            for l in 0..tp_name_size {
                if pyobj_tp_name[l] == 0 {
                    tp_name_size = l;
                    break;
                }
            }
            let custom_tp_name = if tp_name_size > 0 {
                String::from_utf8_lossy(&pyobj_tp_name[0..tp_name_size]).into_owned()
            } else {
                return Err(io::Error::new(io::ErrorKind::Other, "invalid ob_type"));
            };
            let tp_obj = PyObjectNode {
                base_addr,
                ob_type: pyobj_type_addr,
                tp_name: custom_tp_name,
                attrs: Default::default(),
                items: vec![],
                extras: vec![],
                is_type: true,
                is_resolved: true,
            };
            objects.insert(pyobj_type_addr, tp_obj);
            regions.insert(pyobj_type_addr, pyobj_type_region);
            return Ok(children);
        }
    }

    // remove type object from cache if it exists
    let _ = del_node(base_addr, objects, regions);

    // handle var python object
    let var_size: usize = match tp_name_inferred.as_str() {
        "str" | "bytearray" | "bytes" | "list" | "long" | "tuple" => {
            let var_region = process.read_memory(base_addr, size_of::<CPyVarObject>())?;
            let var_view = var_region.view_bytes_as::<CPyVarObject>(0, None)?;
            var_view.ob_size.abs() as usize
        },
        _ => { 0 }
    };

    let obj_size: usize = match tp_name_inferred.as_str() {
        "str" => { size_of::<CPyStringObject>() }
        "bytearray" => { size_of::<CPyByteArrayObject>() }
        "bytes" => { size_of::<CPyBytesObject>() }
        "list" => { size_of::<CPyListObject>() }
        "long" => { size_of::<CPyLongObject>() }
        "tuple" => { size_of::<CPyTupleObject>() }
        "dict" => { size_of::<CPyDictObject>() }
        "bool" => { size_of::<CPyBoolObject>() }
        "float" => { size_of::<CPyFloatObject>() }
        "int" => { size_of::<CPyIntObject>() }
        "NoneType" => { size_of::<CPyObject>() }
        "unicode" => { size_of::<CPyUnicodeObject>() }
        "type" => { size_of::<CPyTypeObject>() }
        _ => { size_of::<CPyCustomObject>() }
    };

    // reload region with new size
    pyobj_region = process.read_memory(base_addr, obj_size + var_size)?;
    let obj = PyObjectNode {
        base_addr,
        ob_type: pyobj_type_addr,
        tp_name: tp_name_inferred.clone(),
        attrs: Default::default(),
        items: vec![],
        extras: vec![],
        is_type: false,
        is_resolved: tp_name_inferred.as_str() == "type",
    };
    objects.insert(base_addr, obj);
    regions.insert(base_addr, pyobj_region);
    resolve_node(base_addr, process, objects, regions)
}

pub fn del_node(
    base_addr: u64,
    objects: &mut HashMap<u64, PyObjectNode>,
    regions: &mut HashMap<u64, MemoryRegion>
) -> io::Result<PyObjectNode>
{
    if !objects.contains_key(&base_addr) {
        return Err(io::Error::new(io::ErrorKind::Other, "invalid base_addr"));
    }
    let obj = objects.remove(&base_addr).unwrap();
    for dedicated_regions in obj.extras.iter() {
        regions.remove(dedicated_regions);
    }
    Ok(obj)
}

pub fn resolve_node(
    addr: u64, process: &Process,
    objects: &mut HashMap<u64, PyObjectNode>, regions: &mut HashMap<u64, MemoryRegion>
) -> io::Result<Vec<u64>>
{
    let node = objects.get_mut(&addr).ok_or(
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Can't find object at 0x{:X} to resolve", addr)
        )
    )?;
    let mut children = Vec::new();
    if node.is_resolved {
        return Ok(children)
    }
    let region = regions.get_mut(&node.base_addr).ok_or(
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Can't find region at 0x{:X}", node.base_addr)
        )
    )?;
    node.is_resolved = true;
    let tp_name =node.tp_name.clone();
    match tp_name.as_str() {
        "dict" => {
            let attr_dict_view = region.view_bytes_as::<CPyDictObject>(0, None)?;
            let mask = attr_dict_view.ma_mask;
            let ma_table = attr_dict_view.ma_table;

            for i in 0..mask+1 {
                if let Ok(entry_region) = process.read_memory(
                    ma_table + (i as usize * size_of::<CPyDictEntry>()) as u64,
                    size_of::<CPyDictEntry>())
                {
                    if let Ok(entry_view) = entry_region.view_bytes_as::<CPyDictEntry>(0, None) {
                        let me_key_addr = entry_view.me_key;
                        let me_value_addr = entry_view.me_value;
                        if me_key_addr == 0 || me_value_addr == 0 {
                            continue
                        }
                        node.attrs.insert(me_key_addr, me_value_addr);
                        children.push(me_value_addr);
                        children.push(me_key_addr);
                    }
                }
            }
        },
        "list" => {
            let list_region = regions.get_mut(&node.base_addr).ok_or(
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Can't find region at 0x{:X}", node.base_addr)
                )
            )?;
            let list_view = list_region.view_bytes_as::<CPyListObject>(0, None)?;
            let ob_size = list_view.ob_base.ob_size;
            children = list_region.view_bytes_as_vec_of::<u64>(
                CPyListObject::<1>::OFFSET_OB_ITEM.offset(),
                ob_size as usize
            )?.iter().map(|&x| *x).collect();
            node.items.extend(children.iter());
        },
        "str" | "bytes" | "long" | "bool" |
        "NoneType" | "int" | "float" | "type" |
        "function" | "weakref" | "tuple" | "" => {
        },
        "unicode" => {
            let unicode_view = region.view_bytes_as::<CPyUnicodeObject>(0, None)?;
            let str_len = unicode_view.length;
            let extra_region = unicode_view.str;
            let new_extra = process.read_memory(extra_region, (str_len as u64 * size_of::<u16>() as u64) as usize)?;
            regions.insert(extra_region, new_extra);
            node.extras.push(extra_region);
        },
        _ => {
            let custom_obj_view = region.view_bytes_as::<CPyCustomObject>(0, None)?;
            let attr_dict_addr = custom_obj_view.attributes;
            let attr_dict_region = process
                .read_memory(attr_dict_addr, size_of::<CPyDictObject>())?;
            let attr_dict_view = attr_dict_region
                .view_bytes_as::<CPyDictObject>(0, None)?;
            let mask = attr_dict_view.ma_mask;
            let ma_table = attr_dict_view.ma_table;

            for i in 0..mask+1 {
                if let Ok(entry_region) = process.read_memory(
                    ma_table + (i as usize * size_of::<CPyDictEntry>()) as u64,
                    size_of::<CPyDictEntry>())
                {
                    if let Ok(entry_view) = entry_region.view_bytes_as::<CPyDictEntry>(0, None) {
                        let me_key_addr = entry_view.me_key;
                        let me_value_addr = entry_view.me_value;
                        if me_key_addr == 0 || me_value_addr == 0 {
                            continue
                        }
                        node.attrs.insert(me_key_addr, me_value_addr);
                        children.push(me_value_addr);
                        children.push(me_key_addr);
                    }
                }
            }
        }
    }
    Ok(children)
}
