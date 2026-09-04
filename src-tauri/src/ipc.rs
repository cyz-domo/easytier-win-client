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
pub fn serve<F>(mut handler: F) -> Result<(), String> where F: FnMut(Request) -> Response {
    use named_pipe::PipeOptions;
    loop {
        let connecting = PipeOptions::new(r"\\.\pipe\EasyTierService").single().map_err(|e| e.to_string())?;
        let mut pipe = connecting.wait().map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(pipe);
        loop {
            match read_request(&mut reader) {
                Ok(req) => { let resp = handler(req); write_response(reader.get_mut(), &resp)?; }
                Err(_) => break,
            }
        }
    }
}
#[cfg(not(windows))] pub fn serve<F>(_handler: F) -> Result<(), String> where F: FnMut(Request) -> Response { Err("Named Pipe IPC is only available on Windows".into()) }
