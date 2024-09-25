use std::collections::HashMap;
use std::io;
use std::rc::{Rc, Weak};
use crate::eve_process::process::{MemoryRegion, Process};


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
    pub child: HashMap<Index, Rc<PyObjectNode>>
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
            ui_root: None
        }
    }
}


impl EVEProcess {
    pub fn search_python_type_type(self) -> io::Result<()> {
        match self.process {
            Some(process) => {
                process.regions.into_iter().map(
                    |region| {
                        for offset in (0..region.size as u64).step_by(8) {
                            let base_addr = region.start + offset;
                            // let data = region.view_bytes_as::<CPyObject>(offset as usize, 8).unwrap();
                            
                            
                            
                        }
                    }
                );
                Ok(())
            },
            None => Err(io::Error::new(io::ErrorKind::Other, "No process opened."))
        }
    }
}