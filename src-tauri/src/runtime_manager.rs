use crate::config_store::{DesiredState, InstanceConfig};
use serde::Serialize;
use std::{collections::HashMap, fs, process::{Child, Command, Stdio}, sync::{Arc, Mutex}};
#[cfg(windows)] use std::os::windows::process::CommandExt;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ObservedState { Stopped, Starting, Running, Stopping, Failed }
#[derive(Clone, Debug, Serialize)]
pub struct InstanceSnapshot { pub id: String, pub desired_state: DesiredState, pub observed_state: ObservedState, pub pid: Option<u32>, pub last_error: Option<String> }

pub struct RuntimeManager { pub children: HashMap<String, Child>, pub errors: HashMap<String, String>, pub core_path: std::path::PathBuf }
pub type SharedRuntime = Arc<Mutex<RuntimeManager>>;

impl RuntimeManager {
    pub fn new(core_path: impl Into<std::path::PathBuf>) -> Self { Self { children: HashMap::new(), errors: HashMap::new(), core_path: core_path.into() } }
    pub fn snapshot(&mut self, cfg: &InstanceConfig) -> InstanceSnapshot {
        let running = self.children.contains_key(&cfg.id);
        let error = self.errors.get(&cfg.id).cloned().or_else(|| cfg.last_error.clone());
        InstanceSnapshot { id: cfg.id.clone(), desired_state: cfg.desired_state, observed_state: if running { ObservedState::Running } else if error.is_some() { ObservedState::Failed } else { ObservedState::Stopped }, pid: self.children.get(&cfg.id).map(Child::id), last_error: error }
    }
    pub fn start(&mut self, cfg: &InstanceConfig) -> Result<InstanceSnapshot, String> {
        if self.children.contains_key(&cfg.id) { return Ok(self.snapshot(cfg)); }
        if !self.core_path.exists() { return Err("core_not_found".into()); }
        let path = std::env::temp_dir().join(format!("easytier-{}.toml", cfg.id));
        fs::write(&path, &cfg.config_toml).map_err(|e| e.to_string())?;
        let mut command = Command::new(&self.core_path);
        command.arg("--config-file").arg(path).arg("--rpc-portal").arg(format!("127.0.0.1:{}", cfg.rpc_port)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)] command.creation_flags(0x08000000);
        let child = command.spawn().map_err(|e| e.to_string())?;
        self.errors.remove(&cfg.id); self.children.insert(cfg.id.clone(), child); Ok(self.snapshot(cfg))
    }
    pub fn stop(&mut self, cfg: &InstanceConfig) -> Result<InstanceSnapshot, String> {
        if let Some(mut child) = self.children.remove(&cfg.id) { child.kill().map_err(|e| e.to_string())?; let _ = child.wait(); }
        Ok(self.snapshot(cfg))
    }
    pub fn stop_all(&mut self) { let ids: Vec<_> = self.children.keys().cloned().collect(); for id in ids { if let Some(mut child) = self.children.remove(&id) { let _ = child.kill(); let _ = child.wait(); } } }
}
