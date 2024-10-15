use crate::eve_process::process::{MemoryRegion, Process};
use crate::eve_process::py_struct::*;
use lazy_static::lazy_static;
use rayon::prelude::*;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash};
use std::io;
use std::rc::{Rc, Weak};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;
use rustc_hash::FxBuildHasher;

lazy_static! {
    static ref _py_types: Vec<&'static str> = vec!["UIRoot"];
}

// static HASHER: FxHasher = FxHasher::default();

#[derive(Debug)]
pub enum Index {
    Name(String),
    Index(usize),
}

#[derive(Debug, Default)]
pub struct PyObjectNode {
    pub base_addr: u64,
    pub region: MemoryRegion,
    pub ob_type: Weak<PyObjectNode>,
    pub tp_name: String,
    pub child: HashMap<Index, Weak<PyObjectNode>>,
    pub extra: HashMap<u64, MemoryRegion>
}

#[derive(Debug)]
pub struct EVEProcess {
    pub process: Process,
    pub objects: HashMap<u64, Rc<PyObjectNode>>,
    pub py_type: u64,
    pub ui_root: u64
}

macro_rules! par_map_regions {
    ($T:ty, $process:expr, $pyobj_filter:expr) => {
        $process
            .regions
            .par_iter()
            .map_with({ &$process }, |s, region| -> Vec<u64> {
                (0..region.size as u64)
                    .step_by(8)
                    .filter_map({
                        |offset| -> Option<u64> {
                            let base_addr = region.start + offset;
                            let data = region.view_bytes_as::<$T>(offset as usize, Some(8)).unwrap();
                            $pyobj_filter(*s, base_addr, data)
                        }
                    })
                    .collect()
            })
            .reduce(|| vec![], |a, b| a.into_iter().chain(b).collect::<Vec<_>>())
            .into_iter()
            .collect()
    };

    ($T:ty, $default:expr, $process:expr, $pyobj_filter:expr) => {
        $process
            .regions
            .par_iter()
            .map_with({ &$process }, |s, region| {
                (0..region.size as u64)
                    .step_by(8)
                    .filter_map({
                        |offset| -> Option<u64> {
                            let base_addr = region.start + offset;
                            let data = region.view_bytes_as::<$T>(offset as usize, 8).unwrap();
                            $pyobj_filter(*s, base_addr, data)
                        }
                    })
                    .collect()
            })
            .reduce(|| $default, |a, b| a.into_iter().chain(b).collect())
            .into_iter()
            .collect();
    };
}

impl PyObjectNode {
    pub fn new_from_memory(base_addr: u64, eve: &mut EVEProcess) -> io::Result<Rc<Self>> {
        if base_addr == 0 {
            return Err(io::Error::new(io::ErrorKind::Other, "invalid base_addr"));
        }
        let mut tp_name_size = 255;
        let mut pyobj_region = eve.process.read_memory(base_addr, size_of::<CPyObject>())?;
        let pyobj_view = pyobj_region.view_bytes_as::<CPyObject>(0, None)?;
        let pyobj_type_addr = pyobj_view.ob_type;

        let mut tp_obj;
        let mut tp_name_inferred;
        // get existing type objects or new one to cache, we assume that no new type object
        // will be created
        if pyobj_type_addr == base_addr {
            if eve.objects.contains_key(&base_addr) {
                return Ok(eve.objects.get(&base_addr).unwrap().clone());
            }
            let pyobj_type_region = eve.process.read_cache(pyobj_type_addr, size_of::<CPyTypeObject>())?;
            let obj = Rc::new_cyclic(|tp| Self {
                base_addr,
                region: pyobj_type_region,
                ob_type: tp.clone(),
                tp_name: "type".to_string(),
                child: Default::default(),
                extra: Default::default(),
            });
            eve.objects.insert(base_addr, obj.clone());
            return Ok(obj);
        } else if eve.objects.contains_key(&pyobj_type_addr) {
            tp_obj = eve.objects.get(&pyobj_type_addr).unwrap().clone();
            tp_name_inferred = tp_obj.tp_name.clone();
        } else {
            let pyobj_type_region = eve.process.read_cache(pyobj_type_addr, size_of::<CPyTypeObject>())?;
            let pyobj_tp_name_addr = pyobj_type_region.view_bytes_as::<CPyTypeObject>(0, None)?.tp_name;
            let pyobj_tp_name_region = eve.process.read_cache(pyobj_tp_name_addr, tp_name_size)?;
            let pyobj_tp_name = &pyobj_tp_name_region.data;
            for l in 0..tp_name_size {
                if pyobj_tp_name[l] == 0 {
                    tp_name_size = l;
                    break;
                }
            }
            tp_name_inferred = if tp_name_size > 0 {
                String::from_utf8_lossy(&pyobj_tp_name[0..tp_name_size]).into_owned()
            } else {
                return Err(io::Error::new(io::ErrorKind::Other, "invalid ob_type"));
            };
            tp_obj = Rc::new_cyclic(|tp| PyObjectNode {
                base_addr: pyobj_type_addr,
                region: pyobj_type_region,
                ob_type: tp.clone(),
                tp_name: String::from_utf8_lossy(&pyobj_tp_name[0..tp_name_size]).into_owned(),
                child: Default::default(),
                extra: HashMap::from([(pyobj_tp_name_addr, eve.process.read_cache(pyobj_tp_name_addr, tp_name_size).unwrap())]),
            });
            eve.objects.insert(pyobj_type_addr, tp_obj.clone());
        }

        // remove type object from cache if it exists
        if eve.objects.contains_key(&base_addr) {
            eve.objects.remove(&base_addr);
        }

        // handle var python object
        let var_size: usize = match tp_name_inferred.as_str() {
            "str" | "bytearray" | "bytes" | "list" | "long" | "tuple" => {
                let var_region = eve.process.read_memory(base_addr, size_of::<CPyVarObject>())?;
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
        pyobj_region = eve.process.read_memory(base_addr, obj_size + var_size)?;
        let obj = Rc::new(Self {
            base_addr,
            region: pyobj_region,
            ob_type: Rc::downgrade(&tp_obj),
            tp_name: tp_name_inferred,
            child: Default::default(),
            extra: Default::default(),
        });
        eve.objects.insert(base_addr, obj.clone());

        Ok(obj)
    }


}

#[profiling::all_functions]
impl EVEProcess {
    pub fn list() -> io::Result<Vec<EVEProcess>> {
        let p: Vec<_> = Process::list(None, Some("*exefile*"), Some("*星战前夜*"))?
            .into_iter()
            .map(|proc| -> EVEProcess {
                let proc = proc.enum_memory_regions();
                let proc = proc.sync_memory_regions();
                EVEProcess {
                    process: proc,
                    objects: Default::default(),
                    py_type: 0,
                    ui_root: 0,
                }
            })
            .collect();
        Ok(p)
    }
    pub fn init(&mut self) -> io::Result<u64> {
        // find python type type candidates,
        // where ob_type should be it's addr and tp_name should be "type"
        let type_candidates: HashSet<_> = par_map_regions!(
            CPyTypeObject,
            self.process,
            ({
                |proc: &Process, base_addr, data: &CPyTypeObject| -> Option<u64> {
                    if data.ob_base.ob_type == base_addr {
                        let tp_name_p = data.tp_name;
                        if let Ok(ref tp_name_bytes) = proc.read_cache(tp_name_p, 4).borrow() {
                            if let Ok(tp_name) = tp_name_bytes.view_bytes(0, 4) {
                                if tp_name.eq(b"type") {
                                    return Some(base_addr);
                                }
                            }
                        }
                    }
                    None
                }
            })
        );
        // find addrs of some python types with type type candidates,
        // can be used to filter out false type candidates
        let mut verified_type_candidates: HashMap<u64, HashMap<&str, u64>> = HashMap::default();
        let mut verified_type_addr = 0u64;
        // verify type candidates, until valid type addr is found
        'candidate: for &tp_candidate in type_candidates.iter() {
            for &tp_name in _py_types.iter() {
                let found = self.search_type(tp_name, Some(tp_candidate))?;
                if found.len() == 0 {
                    debug!(
                        "{} not found for type candidate: {}, skipped.",
                        tp_name, tp_candidate
                    );
                    continue 'candidate;
                } else {
                    if verified_type_candidates.contains_key(&tp_candidate) {
                        let tp_dict = verified_type_candidates.get_mut(&tp_candidate).unwrap();
                        if tp_dict.contains_key(tp_name) {
                            debug!(
                                "{} already found for type candidate: {}, skipped.",
                                tp_name, tp_candidate
                            );
                            continue;
                        }
                        tp_dict.insert(tp_name, found[0]);
                    } else {
                        verified_type_candidates
                            .insert(tp_candidate, HashMap::from([(tp_name, found[0])]));
                    }
                }
            }
            // if all types are found, we can use this type candidate
            if verified_type_candidates.contains_key(&tp_candidate)
                && verified_type_candidates.get(&tp_candidate).unwrap().len()
                    == _py_types.len()
            {
                debug!("Found verified type candidate: {}", tp_candidate);
                self.objects = Default::default();
                let py_type = PyObjectNode::new_from_memory(tp_candidate, self)?;
                self.objects.insert(tp_candidate, py_type.clone());
                self.py_type = tp_candidate;
                for (&tp_name, &tp_addr) in
                    verified_type_candidates.get(&tp_candidate).unwrap().iter()
                {
                    
                    let tp_obj = PyObjectNode::new_from_memory(tp_addr, self)?;
                    self.objects.insert(tp_addr, tp_obj.clone());
                    if tp_name.eq("UIRoot") {
                        self.ui_root = tp_addr;
                    }
                }
                verified_type_addr = tp_candidate;
                break;
            }
        }
        if verified_type_addr != 0 {
            Ok(verified_type_addr)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "Failed to find verified type candidate."))
        }
    }

    pub fn search_type(&self, tp_name: &str, tp_addr: Option<u64>) -> io::Result<Vec<u64>> {
        
        let tp_candidate = tp_addr.unwrap_or(self.py_type);
        if tp_candidate == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "No invalid tp_addr provided."
            ));
        }
        let res = par_map_regions!(
            CPyTypeObject,
            self.process,
            ({
                |proc: &Process, base_addr, data: &CPyTypeObject| -> Option<u64> {
                    if data.ob_base.ob_type == tp_candidate {
                        let tp_name_p = data.tp_name;
                        if let Ok(ref tp_name_bytes) =
                            proc.read_cache(tp_name_p, tp_name.len()).borrow()
                        {
                            if let Ok(tp_name_read) = tp_name_bytes.view_bytes(0, tp_name.len()) {
                                if tp_name.as_bytes().eq(tp_name_read) {
                                    return Some(base_addr);
                                }
                            }
                        }
                    }
                    None
                }
            })
        );
        Ok(res)
    }

    pub fn search_ui_root(&self, tp_addr: Option<u64>) -> io::Result<Vec<u64>> {
        let tp_addr = tp_addr.unwrap_or(self.ui_root);
        if tp_addr == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "No invalid tp_addr provided."
            ))
        }
        let res = par_map_regions!(
            CPyCustomObject,
            self.process,
            ({
                |proc: &Process, base_addr, data: &CPyCustomObject| -> Option<u64> {
                    if data.ob_base.ob_type == tp_addr {
                        let attr_p = data.attributes;
                        if let Ok(ref tp_name_bytes) =
                            proc.read_cache(attr_p, size_of::<CPyDictObject>()).borrow()
                        {
                            if let Ok(attr_dict) = tp_name_bytes.view_bytes_as::<CPyDictObject>(0, None) {
                                if let Ok(attr_dict_data) = proc.read_cache(attr_dict.ob_base.ob_type, size_of::<CPyTypeObject>()).borrow() {
                                    if let Ok(attr_dict_type) = attr_dict_data.view_bytes_as::<CPyTypeObject>(0, None) {
                                        if let Ok(attr_dict_type_name) = proc.read_cache(attr_dict_type.tp_name, 4).borrow() {
                                            if attr_dict_type_name.view_bytes(0, 4).unwrap_or("".as_bytes()).eq("dict".as_bytes()) {
                                                return Some(base_addr);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    None
                }
            })
        );
        Ok(res)
    }
    
    pub fn parse_ui_tree(&mut self, ui_root_addr: u64) -> Option<PyObjectNode> {
        let region = self.process.read_cache(ui_root_addr, size_of::<CPyCustomObject>()).ok()?;
        let py_obj_view = region.view_bytes_as::<CPyCustomObject>(0, None).ok()?;


        todo!()
    }
}
