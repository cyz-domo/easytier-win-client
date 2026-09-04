use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, io::{BufRead, BufReader, Write}, sync::{Arc, Mutex}};
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE: usize = 1024 * 1024;
#[derive(Debug, Deserialize)] pub struct Request { pub protocol_version: u32, pub request_id: String, pub command: String, #[serde(default)] pub payload: Value }
#[derive(Clone, Debug, Serialize, Deserialize)] pub struct Response { pub protocol_version: u32, pub request_id: String, pub ok: bool, pub data: Option<Value>, pub error: Option<ErrorBody> }
#[derive(Clone, Debug, Serialize, Deserialize)] pub struct ErrorBody { pub code: String, pub message: String }
pub fn response(req: &Request, data: Value) -> Response { Response { protocol_version: PROTOCOL_VERSION, request_id: req.request_id.clone(), ok: true, data: Some(data), error: None } }
pub fn error(req: &Request, code: &str, message: impl Into<String>) -> Response { Response { protocol_version: PROTOCOL_VERSION, request_id: req.request_id.clone(), ok: false, data: None, error: Some(ErrorBody { code: code.into(), message: message.into() }) } }
pub fn read_request<R: BufRead>(reader: &mut R) -> Result<Request, String> {
    let mut line = Vec::new();
    let n = reader.read_until(b'\n', &mut line).map_err(|e| e.to_string())?;
    if n == 0 { return Err("eof".into()); }
    if line.len() > MAX_MESSAGE || (!line.contains(&b'\n') && line.len() >= MAX_MESSAGE) { return Err("message_too_large".into()); }
    let request: Request = serde_json::from_slice(&line).map_err(|e| format!("invalid_request: {e}"))?;
    if request.protocol_version != PROTOCOL_VERSION || request.request_id.trim().is_empty() || request.command.trim().is_empty() { return Err("invalid_request: invalid protocol_version, request_id, or command".into()); }
    Ok(request)
}
pub fn write_response<W: Write>(writer: &mut W, response: &Response) -> Result<(), String> { let bytes = serde_json::to_vec(response).map_err(|e| e.to_string())?; if bytes.len() > MAX_MESSAGE { return Err("response_too_large".into()); } writer.write_all(&bytes).and_then(|_| writer.write_all(b"\n")).and_then(|_| writer.flush()).map_err(|e| e.to_string()) }
pub type IdempotencyCache = Arc<Mutex<HashMap<String, Response>>>;
pub fn new_cache() -> IdempotencyCache { Arc::new(Mutex::new(HashMap::new())) }
#[cfg(windows)]
mod windows_security {
    use super::MAX_MESSAGE;
    use std::{fs::File, mem::size_of, os::windows::{ffi::OsStrExt, io::{AsRawHandle, FromRawHandle}}};
    use windows_sys::Win32::{Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree}, Security::{GetTokenInformation, TokenGroups, TokenUser, TOKEN_GROUPS, TOKEN_QUERY, TOKEN_USER}, Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, SDDL_REVISION_1}, Storage::{FileSystem::PIPE_ACCESS_DUPLEX}, System::{Pipes::{ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT}, Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION}}};
    fn wide(s: &str) -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect() }
    fn winerr(ctx: &str) -> String { format!("{ctx}: Windows error {}", unsafe { GetLastError() }) }
    struct LocalAlloc(*mut core::ffi::c_void);
    impl Drop for LocalAlloc { fn drop(&mut self) { if !self.0.is_null() { unsafe { LocalFree(self.0); } } } }
    fn sid_string(sid: *mut core::ffi::c_void) -> Result<String, String> {
        let mut text = core::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 { return Err(winerr("convert SID")); }
        let mut n = 0;
        unsafe { while *text.add(n) != 0 { n += 1; } }
        let value = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, n) });
        unsafe { LocalFree(text as *mut core::ffi::c_void); }
        Ok(value)
    }
    pub fn validate_sid(sid: &str) -> Result<(), String> {
        let mut parsed = core::ptr::null_mut();
        if unsafe { ConvertStringSidToSidW(wide(sid).as_ptr(), &mut parsed) } == 0 { return Err(winerr("invalid interactive user SID")); }
        unsafe { LocalFree(parsed as *mut core::ffi::c_void); }
        Ok(())
    }
    pub fn create_pipe(sid: &str) -> Result<File, String> {
        validate_sid(sid)?;
        let sddl = format!("D:P(A;;GA;;;{})(A;;GA;;;BA)(A;;GA;;;SY)", sid);
        let mut descriptor = core::ptr::null_mut(); let mut len = 0;
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(&sddl).as_ptr(), SDDL_REVISION_1, &mut descriptor, &mut len) } == 0 { return Err(winerr("create named pipe security descriptor")); }
        let _descriptor = LocalAlloc(descriptor);
        let attrs = windows_sys::Win32::Security::SECURITY_ATTRIBUTES { nLength: size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        let handle = unsafe { CreateNamedPipeW(wide(r"\\.\pipe\EasyTierService").as_ptr(), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, MAX_MESSAGE as u32, MAX_MESSAGE as u32, 0, &attrs) };
        if handle == INVALID_HANDLE_VALUE { return Err(winerr("create named pipe")); }
        Ok(unsafe { File::from_raw_handle(handle as _) })
    }
    pub fn connect(pipe: &File) -> Result<(), String> {
        if unsafe { ConnectNamedPipe(pipe.as_raw_handle() as HANDLE, core::ptr::null_mut()) } == 0 && unsafe { GetLastError() } != 535 { return Err(winerr("wait for named pipe client")); } Ok(())
    }
    pub fn allowed(pipe: &File, configured_sid: &str) -> Result<bool, String> {
        let mut pid = 0; if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) } == 0 { return Err(winerr("get client PID")); }
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) }; if process == 0 { return Err(winerr("open client process")); }
        let mut token = 0; let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }; unsafe { CloseHandle(process); }
        if ok == 0 { return Err(winerr("open client token")); }
        let result = token_allowed(token, configured_sid); unsafe { CloseHandle(token); } result
    }
    fn token_allowed(token: HANDLE, configured_sid: &str) -> Result<bool, String> {
        let mut needed = 0; unsafe { GetTokenInformation(token, TokenUser, core::ptr::null_mut(), 0, &mut needed); }
        if needed == 0 { return Err(winerr("query token user size")); }
        let mut buf = vec![0u8; needed as usize]; if unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as _, needed, &mut needed) } == 0 { return Err(winerr("query token user")); }
        let user_sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
        let user_text = sid_string(user_sid)?;
        if user_text == configured_sid || user_text == "S-1-5-32-544" || user_text == "S-1-5-18" { return Ok(true); }
        let mut expected = core::ptr::null_mut(); if unsafe { ConvertStringSidToSidW(wide(configured_sid).as_ptr(), &mut expected) } == 0 { return Err(winerr("parse configured SID")); }
        let match_user = unsafe { windows_sys::Win32::Security::EqualSid(user_sid, expected) != 0 }; unsafe { LocalFree(expected); }
        if match_user { return Ok(true); }
        let mut needed = 0; unsafe { GetTokenInformation(token, TokenGroups, core::ptr::null_mut(), 0, &mut needed); }
        if needed == 0 { return Ok(false); }
        let mut groups_buf = vec![0u8; needed as usize]; if unsafe { GetTokenInformation(token, TokenGroups, groups_buf.as_mut_ptr() as _, needed, &mut needed) } == 0 { return Err(winerr("query token groups")); }
        let groups = unsafe { &*(groups_buf.as_ptr() as *const TOKEN_GROUPS) };
        for i in 0..groups.GroupCount as usize { let group = unsafe { &*groups.Groups.as_ptr().add(i) }; let mut text = core::ptr::null_mut(); if unsafe { ConvertSidToStringSidW(group.Sid, &mut text) } == 0 { return Err(winerr("convert group SID")); } let mut n = 0; unsafe { while *text.add(n) != 0 { n += 1; } } let value = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, n) }); unsafe { LocalFree(text as *mut core::ffi::c_void); } if value == configured_sid || value == "S-1-5-32-544" || value == "S-1-5-18" { return Ok(true); } }
        Ok(false)
    }
}

#[cfg(windows)]
pub fn serve<F>(mut handler: F, interactive_sid: &str) -> Result<(), String> where F: FnMut(Request) -> Response {
    windows_security::validate_sid(interactive_sid)?;
    loop { let mut pipe = windows_security::create_pipe(interactive_sid)?; windows_security::connect(&pipe)?; if !windows_security::allowed(&pipe, interactive_sid)? { continue; } let mut reader = BufReader::new(&mut pipe); loop { match read_request(&mut reader) { Ok(req) => { let resp = handler(req); write_response(reader.get_mut(), &resp)?; }, Err(_) => break } } }
}
#[cfg(not(windows))] pub fn serve<F>(_handler: F) -> Result<(), String> where F: FnMut(Request) -> Response { Err("Named Pipe IPC is only available on Windows".into()) }
