use std::borrow::BorrowMut;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::io;
use std::ffi::{OsString};
use std::fmt::{Debug};
use std::fs::File;
use std::io::Error;
use std::ops::{Deref, DerefMut};
use std::os::windows::ffi::OsStringExt;
use std::rc::{Rc, Weak};
use std::sync::Mutex;
use winapi::shared::minwindef::{DWORD, FALSE, LPARAM, BOOL, TRUE, LPVOID};
use winapi::shared::windef::HWND;
use winapi::shared::ntdef::{HANDLE, NULL};
use winapi::um::processthreadsapi::OpenProcess;
use winapi::um::psapi::{GetProcessImageFileNameW};
use winapi::um::winnt::{MEMORY_BASIC_INFORMATION64, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS, PMEMORY_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ, ULONGLONG};
use winapi::um::winuser::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, GetWindowTextLengthW};
use wildmatch::WildMatch;
use winapi::um::memoryapi::{ReadProcessMemory, VirtualQueryEx};
use tracing::{info, debug, warn};
use rayon::prelude::*;


/// How many ASCII characters to read for a process name at most.
const MAX_PROC_NAME_LEN: usize = 128;
const MAX_PROC_PATH_LEN: usize = 1024;
const MAX_PROC_NUM: usize = 1024;

/// A handle to an opened process.

#[derive(Debug, Clone)]
pub enum ProcessHandle {
    Live(u32),
    File,
    None
}

#[derive(Debug)]
pub struct Process {
    pub pid: u32,
    pub path: String,
    pub title: String,
    pub regions: Vec<MemoryRegion>,
    handle: ProcessHandle,
}

#[derive(Debug)]
pub struct MemoryRegion {
    pub start: u64,
    pub size: u64,
    pub data: Vec::<u8>,
    pub handle: ProcessHandle
}

impl MemoryRegion {
    pub fn new(start: u64, size: u64, handle: ProcessHandle) -> Self {
        MemoryRegion {
            start,
            size,
            data: vec![0; size as usize],
            handle,
        }
    }

    pub fn bound(mut self, handle: ProcessHandle) -> Self {
        self.handle = handle;
        self
    }

    pub fn sync(&mut self) -> io::Result<()> {
        match &self.handle {
            ProcessHandle::Live(handle) => {
                unsafe {
                    if ReadProcessMemory(*handle as HANDLE, self.start as LPVOID, self.data.as_mut_ptr() as LPVOID, self.size as usize, NULL as *mut _) == TRUE {
                        Ok(())
                    } else {
                        Err(io::Error::last_os_error())
                    }
                }
            }
            ProcessHandle::File => {
                todo!("File reading not implemented yet")
            }
            ProcessHandle::None => {
                Err(io::Error::new(io::ErrorKind::Other, "No bounded process."))
            }
        }
    }
}

impl Process {
    pub fn list(pid: Option<u32>, path:Option<&str>, title: Option<&str>) -> io::Result<Vec::<Self>> {
        match list_processes() {
            Err(e) => Err(e),
            Ok(processes) => {
                debug!("{:?} {}", &processes, "Processes found");
                let filtered = processes.into_iter().filter(
                    |proc|
                        (pid.is_none() || proc.pid == pid.unwrap()) &&
                            (path.is_none() || WildMatch::new(path.unwrap()).matches(&proc.path)) &&
                            (title.is_none() || WildMatch::new(title.unwrap()).matches(&proc.title))
                ).collect::<Vec<Self>>();
                if filtered.is_empty() {
                    Err(io::Error::new(io::ErrorKind::NotFound, format!("Process not found (pid={pid:?}, path={path:?}, title={title:?})")))
                } else {
                    Ok(filtered)
                }
            }
        }
    }
    pub fn enum_memory_regions(&mut self) {
        let mut mem_info = MEMORY_BASIC_INFORMATION64 {
            BaseAddress: 0,
            AllocationBase: 0,
            AllocationProtect: 0,
            __alignment1: 0,
            RegionSize: 0,
            State: 0,
            Protect: 0,
            Type: 0,
            __alignment2: 0,
        };
        let mut current_address: LPVOID  = NULL;
        self.regions.clear();

        match self.handle {
            ProcessHandle::Live(handle) => {
                unsafe {
                    while VirtualQueryEx(handle as HANDLE, current_address, &mut mem_info as *mut _ as PMEMORY_BASIC_INFORMATION, size_of::<MEMORY_BASIC_INFORMATION64>()
                    ) == size_of::<MEMORY_BASIC_INFORMATION64>() {
                        if mem_info.State == MEM_COMMIT && mem_info.Protect & PAGE_NOACCESS == 0 && mem_info.Protect & PAGE_GUARD == 0 {
                            self.regions.push(MemoryRegion::new(mem_info.BaseAddress, mem_info.RegionSize, ProcessHandle::Live(handle)))
                        }
                        current_address = (mem_info.BaseAddress + mem_info.RegionSize) as LPVOID;
                    }
                }
            }
            ProcessHandle::File => {}
            ProcessHandle::None => {}
        }
    }

    pub fn sync_memory_regions(&mut self) {
        self.regions.par_iter_mut().for_each(|region| region.sync().unwrap_or(()))
    }

    pub fn get_region_from_address(&self, addr: u64) -> io::Result<(usize, usize)> {
        match self.regions.binary_search_by_key(&addr, |region| region.start) {
            Ok(index) => Ok((index, 0)),
            Err(index) => {
                if index == 0 || index == self.regions.len() {
                    Err(Error::new(io::ErrorKind::InvalidInput, "Address not found in any memory region"))
                } else {
                    let offset = addr - self.regions[index].start;
                    if offset < 0 {
                        Err(Error::new(io::ErrorKind::InvalidInput, "Unknown error, MemoryRegions may not be correctly sorted."))
                    }
                    else if offset > self.regions[index].size {
                        Err(Error::new(io::ErrorKind::InvalidInput, "Address not found in any memory region"))
                    }
                    else {
                        Ok((index, offset as usize))
                    }
                }
            }
        }
    }

    pub fn read_cached(&mut self, addr: u64, size: usize) -> io::Result<Vec<u8>> {
        let (index, offset) = self.get_region_from_address(addr)?;
        let region = &mut self.regions[index];
        Ok(region.data[offset..offset + size].to_vec())
    }

    pub fn read_memory(&mut self, addr: u64, size: usize) -> io::Result<Vec<u8>> {
        match self.handle {
            ProcessHandle::Live(handle) => {
                unsafe {
                    let mut data = vec![0; size];
                    if ReadProcessMemory(handle as HANDLE, addr as LPVOID, data.as_mut_ptr() as LPVOID, size, NULL as *mut _) == TRUE {
                        Ok(data)
                    } else {
                        Err(Error::last_os_error())
                    }
                }
            }
            ProcessHandle::File => {
                todo!("File reading not implemented yet")
            }
            ProcessHandle::None => {
                Err(Error::new(io::ErrorKind::Other, "No process opened."))
            }
        }
    }

}

unsafe extern "system" fn list_processes_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let processes = &mut *(lparam as *mut Vec<Process>);

    // get the process id
    let mut raw_pid: DWORD = 0;
    GetWindowThreadProcessId(hwnd, &mut raw_pid);
    if raw_pid == 0 {
        return TRUE;
    }
    // get the window title
    let title_len: u32 = GetWindowTextLengthW(hwnd) as u32;
    if title_len == 0 {
        return TRUE;
    }
    let mut raw_title: Vec<u16> = vec![0; (title_len + 1) as usize];
    GetWindowTextW(hwnd, raw_title.as_mut_ptr(), MAX_PROC_NAME_LEN as i32);

    // get the process path
    let raw_handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, FALSE, raw_pid);
    if raw_handle == NULL {
        return TRUE;
    }
    let mut raw_path: Vec<u16> = vec![0; MAX_PROC_PATH_LEN];
    let path_len: u32 = GetProcessImageFileNameW(raw_handle, raw_path.as_mut_ptr(), raw_path.len() as u32);
    if path_len != 0 {
        raw_path.set_len(path_len as usize + 1);
    } else { return TRUE; }
    processes.push(
        Process {
            pid: raw_pid,
            path: OsString::from_wide(&raw_path[..path_len as usize]).to_string_lossy().into_owned(),
            title: OsString::from_wide(&raw_title[..title_len as usize]).to_string_lossy().into_owned(),
            regions: vec![],
            handle: ProcessHandle::Live(raw_handle as u32),
        }
    );
    TRUE
}

pub fn list_processes() -> io::Result<Vec<Process>> {
    let mut processes = Vec::<Process>::with_capacity(MAX_PROC_NUM);
    unsafe { EnumWindows(Some(list_processes_callback), &mut processes as *mut Vec<Process> as isize); }
    Ok(processes)
}
