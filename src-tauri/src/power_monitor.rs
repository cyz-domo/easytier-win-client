#[cfg(windows)]
pub mod windows_power {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::{AppHandle, Emitter};

    const DEVICE_NOTIFY_CALLBACK: u32 = 2;
    const PBT_APMRESUMEAUTOMATIC: u32 = 0x0012;
    const PBT_APMRESUMESUSPEND: u32 = 0x0007;
    const PBT_APMSUSPEND: u32 = 0x0004;

    #[repr(C)]
    struct DeviceNotifySubscribeParameters {
        callback: unsafe extern "system" fn(*mut std::ffi::c_void, u32, *mut std::ffi::c_void) -> u32,
        context: *mut std::ffi::c_void,
    }

    extern "system" {
        fn RegisterSuspendResumeNotification(
            h_recipient: *const std::ffi::c_void,
            flags: u32,
        ) -> *mut std::ffi::c_void;
        #[allow(dead_code)]
        fn UnregisterSuspendResumeNotification(handle: *mut std::ffi::c_void) -> i32;
    }

    static REGISTERED: AtomicBool = AtomicBool::new(false);
    static mut APP_HANDLE: Option<AppHandle> = None;

    unsafe extern "system" fn power_callback(
        _context: *mut std::ffi::c_void,
        type_: u32,
        _setting: *mut std::ffi::c_void,
    ) -> u32 {
        if type_ == PBT_APMRESUMEAUTOMATIC || type_ == PBT_APMRESUMESUSPEND {
            eprintln!("[Power] Windows 睡眠唤醒事件 (Type: 0x{:X})，触发网络自愈感知...", type_);
            if let Some(ref app) = APP_HANDLE {
                let _ = app.emit("system-power-resumed", ());
            }
        } else if type_ == PBT_APMSUSPEND {
            eprintln!("[Power] Windows 进入休眠/睡眠 (Type: 0x{:X})", type_);
            if let Some(ref app) = APP_HANDLE {
                let _ = app.emit("system-power-suspended", ());
            }
        }
        0 // ERROR_SUCCESS
    }

    pub fn init_power_monitor(app: AppHandle) {
        if REGISTERED.swap(true, Ordering::SeqCst) {
            return;
        }
        unsafe {
            APP_HANDLE = Some(app);
            let params = DeviceNotifySubscribeParameters {
                callback: power_callback,
                context: std::ptr::null_mut(),
            };
            let handle = RegisterSuspendResumeNotification(
                &params as *const _ as *const std::ffi::c_void,
                DEVICE_NOTIFY_CALLBACK,
            );
            if handle.is_null() {
                eprintln!("[Power] 注册 Windows 电源事件监听失败");
            } else {
                eprintln!("[Power] 已成功注册 Windows 电源状态与睡眠唤醒监听 (DEVICE_NOTIFY_CALLBACK)");
            }
        }
    }
}
