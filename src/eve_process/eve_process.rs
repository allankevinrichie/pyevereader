use crate::eve_process::process::{MemoryRegion, Process};
use crate::eve_process::py_struct::*;
use lazy_static::lazy_static;
use rayon::prelude::*;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::io;
use std::rc::{Rc, Weak};
use tracing::debug;

lazy_static! {
    static ref py_builtin_types: Vec<&'static str> = vec!["UIRoot"];
}

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
    pub child: HashMap<Index, Rc<PyObjectNode>>,
}

#[derive(Debug)]
pub struct EVEProcess {
    pub process: Process,
    pub objects: HashMap<u64, Rc<PyObjectNode>>,
    pub py_type: Rc<PyObjectNode>,
    pub ui_root: Rc<PyObjectNode>,
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
                            let data = region.view_bytes_as::<$T>(offset as usize, 8).unwrap();
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
                    py_type: Default::default(),
                    ui_root: Default::default(),
                }
            })
            .collect();
        Ok(p)
    }
    pub fn init(&mut self) -> Option<u64> {
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
        // find addrs of some python builtin types with type type candidates,
        // can be used to filter out false type candidates
        let mut verified_type_candidates: HashMap<u64, HashMap<&str, u64>> = HashMap::default();
        let mut verified_type_addr = 0u64;
        // verify type candidates, until valid type addr is found
        'candidate: for &tp_candidate in type_candidates.iter() {
            for &tp_name in py_builtin_types.iter() {
                let found = self.search_type(tp_name, Some(tp_candidate));
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
            // if all builtin types are found, we can use this type candidate
            if verified_type_candidates.contains_key(&tp_candidate)
                && verified_type_candidates.get(&tp_candidate).unwrap().len()
                    == py_builtin_types.len()
            {
                debug!("Found verified type candidate: {}", tp_candidate);
                self.objects = Default::default();
                let py_type = Rc::new(PyObjectNode {
                    base_addr: tp_candidate,
                    region: MemoryRegion {
                        start: tp_candidate,
                        size: size_of::<CPyTypeObject>(),
                        data: self
                            .process
                            .read_cache(tp_candidate, size_of::<CPyTypeObject>())
                            .unwrap()
                            .data,
                        handle: self.process.handle,
                    },
                    ob_type: Default::default(),
                    tp_name: "type".to_string(),
                    child: Default::default(),
                });
                self.objects.insert(tp_candidate, py_type.clone());
                for (&tp_name, &tp_addr) in
                    verified_type_candidates.get(&tp_candidate).unwrap().iter()
                {
                    let tp_obj = Rc::new(PyObjectNode {
                        base_addr: tp_addr,
                        region: MemoryRegion {
                            start: tp_addr,
                            size: size_of::<CPyTypeObject>(),
                            data: self
                                .process
                                .read_cache(tp_addr, size_of::<CPyTypeObject>())
                                .unwrap()
                                .data,
                            handle: self.process.handle,
                        },
                        ob_type: Rc::downgrade(&py_type),
                        tp_name: tp_name.to_string(),
                        child: Default::default(),
                    });
                    self.objects.insert(tp_addr, tp_obj);
                }
                verified_type_addr = tp_candidate;
                self.py_type = py_type;
                break;
            }
        }
        if verified_type_addr != 0 {
            Some(verified_type_addr)
        } else {
            None
        }
    }

    pub fn search_type(&self, tp_name: &str, tp_addr: Option<u64>) -> Vec<u64> {
        let tp_candidate = tp_addr.unwrap_or(self.py_type.base_addr);
        par_map_regions!(
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
        )
    }
    
    pub fn search_object(&self, tp_addr: u64, obj_addr: u64) -> Vec<u64> {
        
    }
}
