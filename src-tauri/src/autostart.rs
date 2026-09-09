use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoStartStatus {
    pub enabled: bool,
    pub start_minimized: bool,
}

#[cfg(windows)]
pub fn get_status() -> Result<AutoStartStatus, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
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
        return Ok(AutoStartStatus {
            enabled: false,
            start_minimized: false,
        });
    }

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

    if query_res != ERROR_SUCCESS || val_type != REG_SZ || val_len == 0 {
        unsafe { RegCloseKey(hkey) };
        return Ok(AutoStartStatus {
            enabled: false,
            start_minimized: false,
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
    unsafe { RegCloseKey(hkey) };

    if query_data != ERROR_SUCCESS {
        return Ok(AutoStartStatus {
            enabled: false,
            start_minimized: false,
        });
    }

    let end_idx = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let val_str = OsString::from_wide(&buffer[..end_idx])
        .to_string_lossy()
        .to_string();

    let start_minimized = val_str.contains("--minimized") || val_str.contains("--tray") || val_str.contains("--silent");
    Ok(AutoStartStatus {
        enabled: true,
        start_minimized,
    })
}

#[cfg(not(windows))]
pub fn get_status() -> Result<AutoStartStatus, String> {
    Ok(AutoStartStatus {
        enabled: false,
        start_minimized: false,
    })
}

#[cfg(windows)]
pub fn set_status(enabled: bool, start_minimized: bool) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

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

    if !enabled {
        let _ = unsafe { RegDeleteValueW(hkey, value_name.as_ptr()) };
        unsafe { RegCloseKey(hkey) };
        return Ok(());
    }

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("获取当前可执行文件路径失败: {}", e))?;
    let exe_str = current_exe.to_string_lossy().to_string();

    let cmd_line = if start_minimized {
        format!("\"{}\" --minimized", exe_str)
    } else {
        format!("\"{}\"", exe_str)
    };

    let mut wide_cmd: Vec<u16> = cmd_line.encode_utf16().collect();
    wide_cmd.push(0);

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
    unsafe { RegCloseKey(hkey) };

    if set_res != ERROR_SUCCESS {
        return Err(format!("写入注册表失败 (错误码: {})", set_res));
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn set_status(_enabled: bool, _start_minimized: bool) -> Result<(), String> {
    Err("仅 Windows 支持开机自启".into())
}
