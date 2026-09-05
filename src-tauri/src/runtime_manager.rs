use crate::config_store::{DesiredState, InstanceConfig};
use serde::Serialize;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

pub const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct LogBuffer {
    text: String,
}

impl LogBuffer {
    pub fn append(&mut self, bytes: &[u8]) {
        self.text.push_str(&String::from_utf8_lossy(bytes));
        if self.text.len() > MAX_LOG_BYTES {
            let mut start = self.text.len() - MAX_LOG_BYTES;
            while start < self.text.len() && !self.text.is_char_boundary(start) {
                start += 1;
            }
            self.text.drain(..start);
        }
    }
    pub fn text(&self) -> String {
        self.text.clone()
    }
}

pub type SharedLog = Arc<Mutex<LogBuffer>>;

pub fn spawn_log_reader<R: Read + Send + 'static>(reader: R, log: SharedLog, stderr: bool) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if stderr {
                        let mut prefixed = b"[stderr] ".to_vec();
                        prefixed.extend_from_slice(&line);
                        log.lock().unwrap().append(&prefixed);
                    } else {
                        log.lock().unwrap().append(&line);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ObservedState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}
#[derive(Clone, Debug, Serialize)]
pub struct InstanceSnapshot {
    pub id: String,
    pub name: String,
    pub rpc_port: u16,
    pub desired_state: DesiredState,
    pub observed_state: ObservedState,
    pub pid: Option<u32>,
    pub last_error: Option<String>,
}

pub struct RuntimeManager {
    pub children: HashMap<String, Child>,
    pub errors: HashMap<String, String>,
    pub logs: HashMap<String, SharedLog>,
    pub core_path: std::path::PathBuf,
}
pub type SharedRuntime = Arc<Mutex<RuntimeManager>>;

impl RuntimeManager {
    pub fn new(core_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            children: HashMap::new(),
            errors: HashMap::new(),
            logs: HashMap::new(),
            core_path: core_path.into(),
        }
    }
    pub fn snapshot(&mut self, cfg: &InstanceConfig) -> InstanceSnapshot {
        let running = self.children.contains_key(&cfg.id);
        let error = self
            .errors
            .get(&cfg.id)
            .cloned()
            .or_else(|| cfg.last_error.clone());
        InstanceSnapshot {
            id: cfg.id.clone(),
            name: cfg.name.clone(),
            rpc_port: cfg.rpc_port,
            desired_state: cfg.desired_state,
            observed_state: if running {
                ObservedState::Running
            } else if error.is_some() {
                ObservedState::Failed
            } else {
                ObservedState::Stopped
            },
            pid: self.children.get(&cfg.id).map(Child::id),
            last_error: error,
        }
    }
    pub fn logs(&self, id: &str) -> String {
        self.logs
            .get(id)
            .map(|log| log.lock().unwrap().text())
            .unwrap_or_default()
    }
    pub fn start(&mut self, cfg: &InstanceConfig) -> Result<InstanceSnapshot, String> {
        if self.children.contains_key(&cfg.id) {
            return Ok(self.snapshot(cfg));
        }
        if !self.core_path.exists() {
            return Err("core_not_found".into());
        }
        let path = std::env::temp_dir().join(format!("easytier-{}.toml", cfg.id));
        fs::write(&path, &cfg.config_toml).map_err(|e| e.to_string())?;
        let mut command = Command::new(&self.core_path);
        command
            .arg("--config-file")
            .arg(path)
            .args(crate::portal_args::build_rpc_portal_args(
                cfg.remote_manage_enabled,
                cfg.rpc_port,
                &cfg.rpc_whitelist_cidrs,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let log = self
            .logs
            .entry(cfg.id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(LogBuffer::default())))
            .clone();
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, log.clone(), false);
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, log, true);
        }
        self.errors.remove(&cfg.id);
        self.children.insert(cfg.id.clone(), child);
        for _ in 0..20 {
            if std::net::TcpStream::connect(("127.0.0.1", cfg.rpc_port)).is_ok() {
                return Ok(self.snapshot(cfg));
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        Ok(self.snapshot(cfg))
    }
    pub fn stop(&mut self, cfg: &InstanceConfig) -> Result<InstanceSnapshot, String> {
        if let Some(mut child) = self.children.remove(&cfg.id) {
            child.kill().map_err(|e| e.to_string())?;
            let _ = child.wait();
        }
        Ok(self.snapshot(cfg))
    }
    pub fn stop_all(&mut self) {
        let ids: Vec<_> = self.children.keys().cloned().collect();
        for id in ids {
            if let Some(mut child) = self.children.remove(&id) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
