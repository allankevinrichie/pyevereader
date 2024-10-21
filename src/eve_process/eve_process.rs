use crate::eve_process::process::{MemoryRegion, Process};
use crate::eve_process::py_struct::*;
use crate::eve_process::pyobject_node::*;
use lazy_static::lazy_static;
use rayon::prelude::*;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::io;
use tracing::debug;


lazy_static! {
    static ref _py_types: Vec<&'static str> = vec!["UIRoot"];
}

#[derive(Debug)]
pub struct EVEProcess {
    pub process: Process,
    pub objects: HashMap<u64, PyObjectNode>,
    pub regions: HashMap<u64, MemoryRegion>,
    pub py_type: u64,
    pub ui_root_type: u64,
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

#[profiling::all_functions]
impl EVEProcess {
    pub fn list() -> io::Result<Vec<Self>> {
        let p: Vec<_> = Process::list(None, Some("*exefile*"), Some("*星战前夜*"))?
            .into_iter()
            .map(|proc| -> Self {
                let proc = proc.enum_memory_regions();
                let proc = proc.sync_memory_regions();
                Self {
                    process: proc,
                    objects: Default::default(),
                    regions: Default::default(),
                    py_type: 0,
                    ui_root_type: 0,
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
                self.new_node(tp_candidate)?;
                self.py_type = tp_candidate;
                for (&tp_name, &tp_addr) in
                    verified_type_candidates.get(&tp_candidate).unwrap().iter()
                {
                    
                    self.new_node(tp_addr)?;
                    if tp_name.eq("UIRoot") {
                        self.ui_root_type = tp_addr;
                    }
                }
                verified_type_addr = tp_candidate;
                break;
            }
        }
        if verified_type_addr != 0 {
            let mut ui_root_obj_candidates = self.search_ui_root(None)?;
            match ui_root_obj_candidates.len() { 
                0_usize => {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Failed to find UIRoot object."
                    ))
                },
                1_usize => {
                    Ok(ui_root_obj_candidates[0])
                },
                _ => {
                    ui_root_obj_candidates.sort_by_key(|&x| {
                        - (self.new_node(x).unwrap_or(Vec::new()).len() as i32)
                    });
                    self.ui_root = ui_root_obj_candidates[0];
                    Ok(ui_root_obj_candidates[0])
                }
            }
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
        let tp_addr = tp_addr.unwrap_or(self.ui_root_type);
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

    pub fn new_node(&mut self, base_addr: u64) -> io::Result<Vec<u64>> {
        new_node(base_addr, &self.process, &mut self.objects, &mut self.regions)
    }

    pub fn del_node(&mut self, base_addr: u64) -> io::Result<PyObjectNode> {
        del_node(base_addr, &mut self.objects, &mut self.regions)
    }

    pub fn resolve_node(&mut self, addr: u64) -> io::Result<Vec<u64>> {
        resolve_node(addr, &self.process, &mut self.objects, &mut self.regions)
    }
}
