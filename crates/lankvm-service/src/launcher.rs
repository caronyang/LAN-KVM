//! Launch a SYSTEM session agent into the active console session.
//!
//! The service runs in session 0 and cannot inject input into the user's
//! desktop directly. We duplicate the token of the active session's
//! `winlogon.exe`, request UIAccess, and start our own executable in that
//! session. The child then opens the active input desktop (Default or
//! Winlogon) itself.

use std::ffi::{c_void, OsStr};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use anyhow::{anyhow, Result};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, DuplicateTokenEx, LookupPrivilegeValueW, SetTokenInformation,
    TokenPrimary, TokenUIAccess, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTSGetActiveConsoleSessionId,
};
use windows_sys::Win32::System::SystemServices::MAXIMUM_ALLOWED;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
    TerminateProcess, WaitForSingleObject, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    STARTUPINFOW,
};

pub struct Child {
    handle: HANDLE,
    pub pid: u32,
    pub session_id: u32,
}

impl Child {
    pub fn is_running(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_TIMEOUT }
    }

    pub fn exit_code(&self) -> u32 {
        if self.handle.is_null() {
            return u32::MAX;
        }
        let mut code = 0u32;
        unsafe {
            GetExitCodeProcess(self.handle, &mut code);
        }
        code
    }

    pub fn stop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        unsafe {
            TerminateProcess(self.handle, 1);
            WaitForSingleObject(self.handle, 2_000);
            CloseHandle(self.handle);
        }
        self.handle = null_mut();
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn active_session() -> u32 {
    let id = unsafe { WTSGetActiveConsoleSessionId() };
    if id != u32::MAX && id != 0 {
        return id;
    }

    // At the logon screen (and on some VMs) the API can be unavailable or
    // return 0. Fall back to the session that owns winlogon.exe.
    let fallback = winlogon_session();
    fallback.unwrap_or(id)
}

fn winlogon_session() -> Option<u32> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut sessions = Vec::new();
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        let end = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if name.eq_ignore_ascii_case("winlogon.exe") {
            let mut session = 0u32;
            if unsafe { ProcessIdToSessionId(entry.th32ProcessID, &mut session) } != 0
                && session != 0
            {
                sessions.push(session);
            }
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    sessions.sort_unstable();
    sessions.dedup();
    // Prefer the session with a user's winlogon; normally there is only one.
    sessions.into_iter().min()
}

pub fn enable_privileges() -> Vec<String> {
    let names = [
        "SeDebugPrivilege",
        "SeTcbPrivilege",
        "SeAssignPrimaryTokenPrivilege",
        "SeIncreaseQuotaPrivilege",
    ];
    let mut failed = Vec::new();
    for name in names {
        if !enable_privilege(name) {
            failed.push(name.to_string());
        }
    }
    failed
}

fn enable_privilege(name: &str) -> bool {
    let mut token: HANDLE = null_mut();
    let open = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
    };
    if open == 0 || token.is_null() {
        return false;
    }

    let wide = to_wide(name);
    let mut luid = Default::default();
    let lookup = unsafe { LookupPrivilegeValueW(null(), wide.as_ptr(), &mut luid) };
    if lookup == 0 {
        unsafe { CloseHandle(token) };
        return false;
    }

    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    let adjusted =
        unsafe { AdjustTokenPrivileges(token, 0, &privileges, 0, null_mut(), null_mut()) };
    unsafe { CloseHandle(token) };
    adjusted != 0
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn launch_session(session_id: u32) -> Result<Child> {
    match launch_session_desktop(session_id, "winsta0\\default") {
        Ok(child) => Ok(child),
        Err(default_err) => {
            crate::logger::log(
                "service",
                format!(
                    "launch_session session={session_id} desktop=Default failed: {default_err:#}; trying Winlogon"
                ),
            );
            launch_session_desktop(session_id, "winsta0\\Winlogon").map_err(|winlogon_err| {
                anyhow!(
                    "default desktop failed: {default_err:#}; winlogon desktop failed: {winlogon_err:#}"
                )
            })
        }
    }
}

fn launch_session_desktop(session_id: u32, desktop: &str) -> Result<Child> {
    let token = winlogon_token(session_id)?;

    let exe = std::env::current_exe()?;
    let exe_wide = to_wide(exe.as_os_str());

    let config = crate::config::load();
    let mut command = format!("\"{}\" session --session {}", exe.display(), session_id);
    if let Some(port) = config.client_port {
        command.push_str(&format!(" --client --port {port}"));
    }
    let mut command_wide = to_wide(&command);
    let mut desktop_wide = to_wide(desktop);

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    startup.lpDesktop = desktop_wide.as_mut_ptr();

    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessAsUserW(
            token,
            exe_wide.as_ptr(),
            command_wide.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_NO_WINDOW,
            null(),
            null(),
            &startup,
            &mut process,
        )
    };
    unsafe { CloseHandle(token) };

    if created == 0 {
        return Err(anyhow!(
            "CreateProcessAsUserW failed: {}",
            io::Error::last_os_error()
        ));
    }
    unsafe { CloseHandle(process.hThread) };

    crate::logger::log(
        "service",
        format!(
            "launch_session session={session_id} desktop={desktop} pid={} OK",
            process.dwProcessId
        ),
    );

    Ok(Child {
        handle: process.hProcess,
        pid: process.dwProcessId,
        session_id,
    })
}

fn winlogon_token(session_id: u32) -> Result<HANDLE> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(anyhow!(
            "CreateToolhelp32Snapshot failed: {}",
            io::Error::last_os_error()
        ));
    }

    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

    let mut found_pid = None;
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        let end = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if name.eq_ignore_ascii_case("winlogon.exe") {
            let mut entry_session = 0u32;
            let ok = unsafe { ProcessIdToSessionId(entry.th32ProcessID, &mut entry_session) };
            if ok != 0 && entry_session == session_id {
                found_pid = Some(entry.th32ProcessID);
                break;
            }
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };

    let pid = found_pid.ok_or_else(|| anyhow!("winlogon.exe not found in session {session_id}"))?;
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(anyhow!(
            "OpenProcess(winlogon) failed: {}",
            io::Error::last_os_error()
        ));
    }

    let mut token: HANDLE = null_mut();
    let opened = unsafe {
        OpenProcessToken(
            process,
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
            &mut token,
        )
    };
    unsafe { CloseHandle(process) };
    if opened == 0 || token.is_null() {
        return Err(anyhow!(
            "OpenProcessToken(winlogon) failed: {}",
            io::Error::last_os_error()
        ));
    }

    let mut primary: HANDLE = null_mut();
    let duplicated = unsafe {
        DuplicateTokenEx(
            token,
            MAXIMUM_ALLOWED,
            null(),
            2, // SecurityImpersonation
            TokenPrimary,
            &mut primary,
        )
    };
    unsafe { CloseHandle(token) };
    if duplicated == 0 || primary.is_null() {
        return Err(anyhow!(
            "DuplicateTokenEx failed: {}",
            io::Error::last_os_error()
        ));
    }

    // UIAccess may fail on an unsigned binary; SYSTEM integrity is still
    // sufficient for the secure desktop in the P0 path. Log it so a failed
    // password-field injection can be diagnosed from the service log.
    let mut ui_access: u32 = 1;
    let ui_access_result = unsafe {
        SetTokenInformation(
            primary,
            TokenUIAccess,
            &mut ui_access as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    crate::logger::log(
        "service",
        format!(
            "TokenUIAccess={} err={}",
            ui_access_result != 0,
            io::Error::last_os_error()
        ),
    );

    Ok(primary)
}

fn to_wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    let mut wide: Vec<u16> = value.as_ref().encode_wide().collect();
    wide.push(0);
    wide
}
