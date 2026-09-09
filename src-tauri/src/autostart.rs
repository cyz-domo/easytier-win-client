use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoStartStatus {
    pub enabled: bool,
    pub start_minimized: bool,
    pub path_mismatch: bool,
    pub registered_path: Option<String>,
}

#[cfg(windows)]
struct RegKeyGuard(windows_sys::Win32::System::Registry::HKEY);

#[cfg(windows)]
impl Drop for RegKeyGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                windows_sys::Win32::System::Registry::RegCloseKey(self.0);
            }
        }
    }
}

#[cfg(windows)]
pub fn get_status() -> Result<AutoStartStatus, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_EXPAND_SZ, REG_SZ,
    };

    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16()
        .collect();
    let value_name: Vec<u16> = "EasyTierWinClient\0".encode_utf16().collect();

    let mut hkey: HKEY = 0;
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        )
    };

    if status != ERROR_SUCCESS {
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(AutoStartStatus {
                enabled: false,
                start_minimized: false,
                path_mismatch: false,
                registered_path: None,
            });
        }
        if status == ERROR_ACCESS_DENIED {
            return Err("访问注册表 Run 键被拒绝，缺少读取权限".into());
        }
        return Err(format!("打开注册表 Run 键失败 (错误码: {})", status));
    }

    let _guard = RegKeyGuard(hkey);

    let mut val_type: u32 = 0;
    let mut val_len: u32 = 0;
    let query_res = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            std::ptr::null_mut(),
            &mut val_len,
        )
    };

    if query_res == ERROR_FILE_NOT_FOUND || val_len == 0 {
        return Ok(AutoStartStatus {
            enabled: false,
            start_minimized: false,
            path_mismatch: false,
            registered_path: None,
        });
    }

    if query_res != ERROR_SUCCESS {
        if query_res == ERROR_ACCESS_DENIED {
            return Err("读取自启动注册表项被拒绝".into());
        }
        return Err(format!("查询注册表失败 (错误码: {})", query_res));
    }

    if val_type != REG_SZ && val_type != REG_EXPAND_SZ {
        return Ok(AutoStartStatus {
            enabled: false,
            start_minimized: false,
            path_mismatch: false,
            registered_path: None,
        });
    }

    let mut buffer: Vec<u16> = vec![0; (val_len as usize / 2) + 1];
    let query_data = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            buffer.as_mut_ptr() as *mut u8,
            &mut val_len,
        )
    };

    if query_data != ERROR_SUCCESS {
        return Err(format!("读取注册表数据失败 (错误码: {})", query_data));
    }

    let end_idx = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let raw_cmd = OsString::from_wide(&buffer[..end_idx]).to_string_lossy().to_string();
    let trimmed = raw_cmd.trim();

    if trimmed.is_empty() {
        return Ok(AutoStartStatus {
            enabled: false,
            start_minimized: false,
            path_mismatch: false,
            registered_path: None,
        });
    }

    // 解析注册表命令行中的可执行文件路径与后续参数
    let (exe_path_in_reg, args_str) = if trimmed.starts_with('"') {
        if let Some(second_quote) = trimmed[1..].find('"') {
            let path = &trimmed[1..1 + second_quote];
            let rest = &trimmed[1 + second_quote + 1..];
            (path, rest)
        } else {
            (trimmed, "")
        }
    } else {
        match trimmed.find(' ') {
            Some(space_idx) => (&trimmed[..space_idx], &trimmed[space_idx + 1..]),
            None => (trimmed, ""),
        }
    };

    // 精确匹配命令行参数，防止路径含有 `--minimized` 触发假阳性
    let args_lower = args_str.to_lowercase();
    let start_minimized = args_lower
        .split_whitespace()
        .any(|arg| arg == "--minimized" || arg == "--tray" || arg == "--silent");

    // 检测便携版移动或当前 exe 与注册表路径是否一致
    let current_exe = std::env::current_exe().ok();
    let path_mismatch = if let Some(ref cur) = current_exe {
        let cur_str = cur.to_string_lossy();
        !exe_path_in_reg.eq_ignore_ascii_case(cur_str.as_ref())
    } else {
        false
    };

    Ok(AutoStartStatus {
        enabled: true,
        start_minimized,
        path_mismatch,
        registered_path: Some(exe_path_in_reg.to_string()),
    })
}

#[cfg(not(windows))]
pub fn get_status() -> Result<AutoStartStatus, String> {
    Ok(AutoStartStatus {
        enabled: false,
        start_minimized: false,
        path_mismatch: false,
        registered_path: None,
    })
}

#[cfg(windows)]
pub fn set_status(enabled: bool, start_minimized: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    // 1. 在操作注册表前先获取当前路径，避免异常提早返回导致句柄泄漏
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("获取当前可执行文件路径失败: {}", e))?;

    // 2. 原生宽字符构建完整命令行，避免 UTF-8 有损转换损坏特殊路径
    let mut wide_cmd: Vec<u16> = Vec::new();
    wide_cmd.push('"' as u16);
    wide_cmd.extend(current_exe.as_os_str().encode_wide());
    wide_cmd.push('"' as u16);

    if start_minimized {
        wide_cmd.extend(" --minimized".encode_utf16());
    }
    wide_cmd.push(0);

    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16()
        .collect();
    let value_name: Vec<u16> = "EasyTierWinClient\0".encode_utf16().collect();

    let mut hkey: HKEY = 0;
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null_mut(),
            &mut hkey,
            std::ptr::null_mut(),
        )
    };

    if status != ERROR_SUCCESS {
        return Err(format!("无法打开注册表 Run 键 (错误码: {})", status));
    }

    let _guard = RegKeyGuard(hkey);

    if !enabled {
        let _ = unsafe { RegDeleteValueW(hkey, value_name.as_ptr()) };
        return Ok(());
    }

    let set_res = unsafe {
        RegSetValueExW(
            hkey,
            value_name.as_ptr(),
            0,
            REG_SZ,
            wide_cmd.as_ptr() as *const u8,
            (wide_cmd.len() * 2) as u32,
        )
    };

    if set_res != ERROR_SUCCESS {
        return Err(format!("写入注册表失败 (错误码: {})", set_res));
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn set_status(_enabled: bool, _start_minimized: bool) -> Result<(), String> {
    Err("仅 Windows 支持开机自启".into())
}
