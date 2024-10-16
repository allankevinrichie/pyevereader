use rayon::prelude::*;
use std::ffi::OsString;
use std::fmt::Debug;
use std::{io, result};
use std::io::Error;
use std::num::NonZeroUsize;
use std::os::windows::ffi::OsStringExt;
use lazy_static::lazy_static;
use tracing::{debug, info, warn};
use wildmatch::WildMatch;
use winapi::shared::minwindef::{BOOL, DWORD, FALSE, LPARAM, LPVOID, TRUE};
use winapi::shared::ntdef::{HANDLE, NULL};
use winapi::shared::windef::HWND;
use winapi::um::memoryapi::{ReadProcessMemory, VirtualQueryEx};
use winapi::um::processthreadsapi::OpenProcess;
use winapi::um::psapi::GetProcessImageFileNameW;
use winapi::um::winnt::{MEMORY_BASIC_INFORMATION64, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PMEMORY_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
use winapi::um::winuser::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
};
use lru::LruCache;
use std::sync::Mutex;
use winapi::um::sysinfoapi::{GetSystemInfo, SYSTEM_INFO};

/// How many ASCII characters to read for a process name at most.
const MAX_PROC_NAME_LEN: usize = 128;
const MAX_PROC_PATH_LEN: usize = 1024;
const MAX_PROC_NUM: usize = 1024;
const MEMORY_MAP_CACHE_SIZE: usize = 1<<6;



lazy_static!{
    static ref _memory_map_cache: Mutex<LruCache::<u64, (usize, usize)>> = 
    Mutex::new(LruCache::new(NonZeroUsize::new(MEMORY_MAP_CACHE_SIZE).unwrap()));
}


/// A handle to an opened process.

#[derive(Debug, Clone, Copy, Default)]
pub enum ProcessHandle {
    Live(u32),
    File,
    #[default]
    None,
}

#[derive(Debug)]
pub struct Process {
    pub pid: u32,
    pub path: String,
    pub title: String,
    pub regions: Vec<MemoryRegion>,
    pub(crate) handle: ProcessHandle,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryRegion {
    pub start: u64,
    pub size: usize,
    pub data: Vec<u8>,
    pub handle: ProcessHandle,
}

#[profiling::all_functions]
impl MemoryRegion {
    pub fn new(start: u64, size: usize, handle: ProcessHandle, data: Option<Vec<u8>>) -> io::Result<Self> {
        Ok(MemoryRegion {
            start,
            size,
            data: data.unwrap_or(vec![0; size]),
            handle,
        })
    }

    pub fn bound(mut self, handle: ProcessHandle) -> io::Result<Self> {
        self.handle = handle;
        Ok(self)
    }

    pub fn sync(mut self) -> Result<Self, (Self, Error)> {
        if let ProcessHandle::Live(h) = self.handle {
            unsafe {
                if ReadProcessMemory(
                    h as HANDLE,
                    self.start as LPVOID,
                    self.data.as_mut_ptr() as LPVOID,
                    self.size as usize,
                    NULL as *mut _,
                ) == TRUE
                {
                    Ok(self)
                } else {
                    Err((self, Error::last_os_error()))
                }
            }
        } else {
            Err((self, Error::new(io::ErrorKind::InvalidInput, "Invalid handle")))
        }
    }
    
    pub fn read_bytes(&self, offset: usize, size: usize) -> io::Result<Self> {
        if offset + size > self.size {
            Err(Error::new(io::ErrorKind::InvalidInput, "Invalid offset or size"))
        } else { 
            MemoryRegion::new(
                self.start + offset as u64,
                size,
                self.handle,
                Some(self.data[offset..offset + size].to_vec()),
            )
        }
    }
    
    pub fn view_bytes(&self, offset: usize, size: usize) -> io::Result<&[u8]> {
        if offset + size > self.size {
            Err(Error::new(io::ErrorKind::InvalidInput, "Invalid offset or size"))
        } else { 
            Ok(&self.data[offset..offset + size])
        }
    }
    
    pub fn view_bytes_as<T>(&self, offset: usize, size: Option<usize>) -> io::Result<&T> {
        let size = size.unwrap_or(size_of::<T>());
        if offset + size > self.size {
            Err(Error::new(io::ErrorKind::InvalidInput, "Invalid offset or size"))
        } else { 
            Ok(unsafe { (self.data[offset..offset + size].as_ptr() as *const T).as_ref().unwrap() })
        }
    }
    
    pub fn view_bytes_as_vec_of<T: Clone>(&self, offset: usize, size: usize) -> io::Result<Vec<&T>> {
        if offset + size > self.size {
            Err(Error::new(io::ErrorKind::InvalidInput, "Invalid offset or size"))
        } else {
            let v: Vec::<&T>;
            Ok(unsafe {
                let t: Vec<_> = self.data[offset..offset + size]
                    .into_iter()
                    .step_by(size_of::<T>())
                    .map(|x| (std::ptr::from_ref(x) as *const T).as_ref().unwrap())
                    .collect();
                t
            })
        }
    }
}

#[profiling::all_functions]
impl Process {
    pub fn list(
        pid: Option<u32>,
        path: Option<&str>,
        title: Option<&str>,
    ) -> io::Result<Vec<Self>> {
        match list_processes() {
            Err(e) => Err(e),
            Ok(processes) => {
                debug!("{:?} {}", &processes, "Processes found");
                let filtered = processes
                    .into_iter()
                    .filter(|proc| {
                        (pid.is_none() || proc.pid == pid.unwrap())
                            && (path.is_none() || WildMatch::new(path.unwrap()).matches(&proc.path))
                            && (title.is_none()
                                || WildMatch::new(title.unwrap()).matches(&proc.title))
                    })
                    .collect::<Vec<Self>>();
                if filtered.is_empty() {
                    Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Process not found (pid={pid:?}, path={path:?}, title={title:?})"),
                    ))
                } else {
                    Ok(filtered)
                }
            }
        }
    }
    pub fn enum_memory_regions(mut self) -> Self {
        let mut sysinfo: SYSTEM_INFO = unsafe { std::mem::zeroed() };
        unsafe { GetSystemInfo(&mut sysinfo)}
        let min_addr = sysinfo.lpMinimumApplicationAddress as u64;
        let max_addr = sysinfo.lpMaximumApplicationAddress as u64;
        let step = 256 * (1 << 20);
        let batch_size = step * 256;
        let num_batches = ((max_addr - min_addr + 1) / batch_size);
        let mut regions_list = Vec::with_capacity(num_batches as usize);
        for i in 0..num_batches {
            let batch_min_addr = i * batch_size + min_addr;
            let batch_max_addr = (i + 1) * batch_size + min_addr;
            let range: Vec<u64> = (batch_min_addr..batch_max_addr).step_by(step as usize).collect();
            let sub_regions: Vec<Vec<MemoryRegion>> = range.into_par_iter().filter_map(
                |start: u64| -> Option<Vec<MemoryRegion>> {
                    let regions = self.enum_memory_regions_in_range(start, start + step as u64);
                    if regions.is_empty() {
                        return None;
                    } else {
                        return Some(regions);
                    }
                }
            ).collect();
            regions_list.push(sub_regions);
        }

        self.regions = regions_list.into_par_iter().filter(|x| !x.is_empty()).reduce(
            || Vec::new(),
            |mut acc, x| {
                acc.extend(x);
                acc
            }
        ).into_par_iter().filter(|x| !x.is_empty()).reduce(
            || Vec::new(),
            |mut acc, x| {
                acc.extend(x);
                acc
            }
        );
        self.regions.sort_by_key(|x| x.start);
        self
    }
    
    fn enum_memory_regions_in_range(&self, start: u64, end: u64) -> Vec<MemoryRegion> {
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
        let mut regions = Vec::new();
        let mut current_address: LPVOID = start as LPVOID;
        match self.handle {
            ProcessHandle::Live(handle) => unsafe {
                while current_address < end as LPVOID && VirtualQueryEx(
                    handle as HANDLE,
                    current_address,
                    &mut mem_info as *mut _ as PMEMORY_BASIC_INFORMATION,
                    size_of::<MEMORY_BASIC_INFORMATION64>(),
                ) == size_of::<MEMORY_BASIC_INFORMATION64>()
                {
                    if mem_info.State == MEM_COMMIT
                        && mem_info.Protect & PAGE_NOACCESS == 0
                        && mem_info.Protect & PAGE_GUARD == 0
                        && mem_info.Protect & (PAGE_READONLY | PAGE_READWRITE) != 0
                    {
                        regions.push(MemoryRegion::new(
                            mem_info.BaseAddress,
                            mem_info.RegionSize as usize,
                            ProcessHandle::Live(handle),
                            None,
                        ).unwrap())
                    }
                    current_address = (mem_info.BaseAddress + mem_info.RegionSize) as LPVOID;
                }
                regions
            },
            ProcessHandle::File => {regions}
            ProcessHandle::None => {regions}
        }
    }

    pub fn sync_memory_regions(mut self) -> Self {
        self.regions = self.regions
            .into_par_iter()
            .filter_map(|region| {
                region.sync().ok()
            }).collect();
        self
    }

    pub fn get_region_from_address(&self, addr: u64) -> io::Result<(usize, usize)> {
        if let Some(&res) = _memory_map_cache.lock().unwrap().get(&addr) {
            return Ok(res);
        }
        let res = match self.regions.binary_search_by_key(&addr, |region| region.start)
        {
            Ok(index) => Ok((index, 0)),
            Err(index) => {
                if index == 0 || index == self.regions.len() {
                    Err(Error::new(
                        io::ErrorKind::InvalidInput,
                        "Address not found in any memory region",
                    ))
                } else {
                    let index = index - 1;
                    let offset = addr - self.regions[index].start;
                    if addr < self.regions[index].start {
                        Err(Error::new(
                            io::ErrorKind::InvalidInput,
                            "Unknown error, MemoryRegions may not be correctly sorted.",
                        ))
                    } else if offset > self.regions[index].size as u64 {
                        Err(Error::new(
                            io::ErrorKind::InvalidInput,
                            "Address not found in any memory region",
                        ))
                    } else {
                        Ok((index, offset as usize))
                    }
                }
            }
        };
        match res {
            Ok((index, offset)) => {
                _memory_map_cache.lock().unwrap().put(addr, (index, offset));
                Ok((index, offset))
            }
            Err(e) => Err(e),
        }
    }

    pub fn read_cache(&self, addr: u64, size: usize) -> io::Result<MemoryRegion> {
        let (index, offset) = self.get_region_from_address(addr)?;
        self.regions.get(index).unwrap().read_bytes(offset, size)
    }

    pub fn read_memory(&self, addr: u64, size: usize) -> io::Result<MemoryRegion> {
        match self.handle {
            ProcessHandle::Live(handle) => unsafe {
                let mut data = vec![0; size];
                if ReadProcessMemory(
                    handle as HANDLE,
                    addr as LPVOID,
                    data.as_mut_ptr() as LPVOID,
                    size,
                    NULL as *mut _,
                ) == TRUE
                {
                    Ok(MemoryRegion {
                        start: addr,
                        size,
                        data,
                        handle: self.handle,
                    })
                } else {
                    Err(Error::last_os_error())
                }
            },
            ProcessHandle::File => {
                todo!("File reading not implemented yet")
            }
            ProcessHandle::None => Err(Error::new(io::ErrorKind::Other, "No process opened.")),
        }
    }
}

#[profiling::function]
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
    let path_len: u32 =
        GetProcessImageFileNameW(raw_handle, raw_path.as_mut_ptr(), raw_path.len() as u32);
    if path_len != 0 {
        raw_path.set_len(path_len as usize + 1);
    } else {
        return TRUE;
    }
    processes.push(Process {
        pid: raw_pid,
        path: OsString::from_wide(&raw_path[..path_len as usize])
            .to_string_lossy()
            .into_owned(),
        title: OsString::from_wide(&raw_title[..title_len as usize])
            .to_string_lossy()
            .into_owned(),
        regions: vec![],
        handle: ProcessHandle::Live(raw_handle as u32),
    });
    TRUE
}

#[profiling::function]
pub fn list_processes() -> io::Result<Vec<Process>> {
    let mut processes = Vec::<Process>::with_capacity(MAX_PROC_NUM);
    unsafe {
        EnumWindows(
            Some(list_processes_callback),
            &mut processes as *mut Vec<Process> as isize,
        );
    }
    Ok(processes)
}
