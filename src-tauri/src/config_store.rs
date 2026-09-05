use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub config_toml: String,
    pub rpc_port: u16,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub desired_state: DesiredState,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DesiredState {
    #[default]
    Stopped,
    Running,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServiceState {
    #[serde(default)]
    pub instances: Vec<InstanceConfig>,
}

pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("EasyTier")
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join("easytier")
    }
}

pub fn load(path: &Path) -> Result<ServiceState, String> {
    if !path.exists() {
        return Ok(ServiceState::default());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid configuration: {e}"))
}

pub fn save(path: &Path, value: &ServiceState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&tmp, path)
        .or_else(|_| {
            let _ = fs::remove_file(path);
            fs::rename(&tmp, path)
        })
        .map_err(|e| e.to_string())
}

pub fn state_path() -> PathBuf {
    data_dir().join("instances.json")
}
