// The GUI must build as a windowed app; this attribute only takes effect in
// the crate that defines `fn main`, so it lives here rather than in lib.rs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Memory and background performance optimizations for WebView2:
    // 1. Limit V8 heap space so garbage collection runs aggressively
    // 2. Limit renderer processes to 1
    // 3. Keep native window occlusion calculation and backgrounding enabled so
    //    the GPU process and renderer sleep when hidden in the tray (0% CPU/GPU).
    #[cfg(windows)]
    {
        // If a local "webview2" directory exists alongside the executable (e.g. in offline standalone portable builds),
        // instruct WebView2 to use it directly without requiring system installation or admin rights.
        if std::env::var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").is_err() {
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(exe_dir) = exe_path.parent() {
                    let local_wv2 = exe_dir.join("webview2");
                    if local_wv2.is_dir() && local_wv2.join("msedgewebview2.exe").is_file() {
                        std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", local_wv2);
                    }
                }
            }
        }

        let existing = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
        let custom_args = "--js-flags=\"--max-old-space-size=64\" --renderer-process-limit=1 --disable-features=TranslateUI,BlinkGenPropertyTrees,SpareRendererForSitePerProcess";
        let new_args = if existing.is_empty() {
            custom_args.to_string()
        } else {
            format!("{} {}", existing, custom_args)
        };
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", new_args);
    }

    easytier_win_client_lib::run();
}
