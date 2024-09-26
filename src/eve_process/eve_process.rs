use crate::eve_process::process::{MemoryRegion, Process};
use crate::eve_process::py_struct::*;
use rayon::prelude::*;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::io;
use std::rc::{Rc, Weak};
use std::sync::{Mutex, RwLock};
use lazy_static::lazy_static;

lazy_static! {
    static ref py_builtin_types: RwLock::<HashMap<&'static str, Option<u64>>> = {
        let mut m = RwLock::new(HashMap::new());
        m.write().unwrap().insert("int", None);
        m.write().unwrap().insert("bool", None);
        m.write().unwrap().insert("float", None);
        m.write().unwrap().insert("str", None);
        m.write().unwrap().insert("unicode", None);
        m.write().unwrap().insert("list", None);
        m.write().unwrap().insert("tuple", None);
        m.write().unwrap().insert("dict", None);
        m.write().unwrap().insert("NoneType", None);
        m
    };
}

#[derive(Debug)]
pub enum Index {
    Name(String),
    Index(usize),
}

#[derive(Debug)]
pub struct PyObjectNode {
    pub region: MemoryRegion,
    pub ob_type: Weak<PyObjectNode>,
    pub tp_name: String,
    pub child: HashMap<Index, Rc<PyObjectNode>>,
}

pub struct EVEProcess {
    pub process: Option<Process>,
    pub objects: Option<Vec<Rc<PyObjectNode>>>,
    pub py_type: Option<Rc<PyObjectNode>>,
    pub ui_root: Option<Rc<PyObjectNode>>
}

impl Default for EVEProcess {
    fn default() -> Self {
        EVEProcess {
            process: None,
            objects: None,
            py_type: None,
            ui_root: None,
        }
    }
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
    pub fn search_python_type_type(self) -> io::Result<()> {
        match self.process {
            Some(ref process) => {
                let type_candidates: HashSet<_> = par_map_regions!(CPyTypeObject, process, ({
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
                }));
                let verified_type_candidates = py_builtin_types.read().unwrap()
                    .iter()
                    .map(|(&tp_name, _)| -> HashMap<u64, Vec<(&str, u64)>> {
                        let found: Vec<_> = type_candidates.iter().filter_map(|&tp_candidate| -> Option<(u64, u64)> {
                            let found: Vec<u64> = par_map_regions!(CPyTypeObject, process, ({
                                |proc: &Process, base_addr, data: &CPyTypeObject| -> Option<u64> {
                                    if data.ob_base.ob_type == tp_candidate {
                                        let tp_name_p = data.tp_name;
                                        if let Ok(ref tp_name_bytes) = proc.read_cache(tp_name_p, tp_name.len()).borrow() {
                                            if let Ok(tp_name) = tp_name_bytes.view_bytes(0, tp_name.len()) {
                                                if tp_name.eq(tp_name) {
                                                    return Some(base_addr);
                                                }
                                            }
                                        }
                                    }
                                    None
                                }
                            }));
                            if found.len() > 0 {
                                Some((tp_candidate, *found.first().unwrap()))
                            } else {
                                None
                            }
                        }).collect();
                        let mut res: HashMap<u64, Vec<(&str, u64)>> = HashMap::new();
                        for (tp_candidate, _) in &found {
                            res.insert(*tp_candidate, Vec::with_capacity(found.len()));
                        }
                        for (tp_candidate, base_addr) in &found {
                            res.get_mut(tp_candidate).unwrap().push((tp_name, *base_addr));
                        }
                        res
                    })
                    .reduce(
                        |a, b| {
                            a.keys().chain(b.keys()).collect::<HashSet<_>>().iter().map(|&&k| {
                                let mut v: Vec<(&str, u64)> = vec![];
                                v.extend(a.get(&k).unwrap_or(&vec![]));
                                v.extend(b.get(&k).unwrap_or(&vec![]));
                                (k, v)
                            }).collect()
                        }
                    ).ok_or(io::Error::new(io::ErrorKind::NotFound, "No valid py builtin types found."));
                match verified_type_candidates {
                    Ok(verified_type_candidates) => {
                        let mut res = verified_type_candidates.iter().map(|(tp_addr, founds)| {
                            (tp_addr, founds.len())
                        }).collect::<Vec<_>>();
                        res.sort_by_key(|&(_, count)| count);
                        println!("found: {:?}", res);
                        return Ok(());
                    },
                    Err(e) => Err(e),
                }
            }
            None => Err(io::Error::new(io::ErrorKind::Other, "No process opened.")),
        }
    }
}
