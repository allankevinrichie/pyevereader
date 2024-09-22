use std::io;
use std::ffi::{OsString};
use std::fmt::Debug;
use std::os::windows::ffi::OsStringExt;
use winapi::shared::minwindef::{DWORD, FALSE, LPARAM, BOOL, TRUE};
use winapi::shared::windef::HWND;
use winapi::shared::ntdef::{HANDLE, NULL};
use winapi::um::processthreadsapi::OpenProcess;
use winapi::um::psapi::{GetProcessImageFileNameW};
use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
use winapi::um::winuser::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, GetWindowTextLengthW};
use wildmatch::WildMatch;


/// How many ASCII characters to read for a process name at most.
const MAX_PROC_NAME_LEN: usize = 128;
const MAX_PROC_PATH_LEN: usize = 1024;
const MAX_PROC_NUM: usize = 1024;

/// A handle to an opened process.
#[derive(Debug)]
pub struct Process {
    pub(crate) pid: u32,
    pub(crate) path: String,
    pub(crate) title: String,
}

unsafe extern "system" fn list_processes_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let processes = &mut *(lparam as *mut Vec<Process>);
    unsafe {
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
                title: OsString::from_wide(&raw_title[..title_len as usize]).to_string_lossy().into_owned()
            }
        )
    }
    TRUE
}

pub fn list_processes() -> io::Result<Vec<Process>> {
    let mut processes = Vec::<Process>::with_capacity(MAX_PROC_NUM);
    unsafe { EnumWindows(Some(list_processes_callback), &mut processes as *mut Vec<Process> as isize); }
    Ok(processes)
}
