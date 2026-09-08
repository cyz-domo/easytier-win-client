use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

static CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn reset_cancel() {
    CANCEL_REQUESTED.store(false, Ordering::SeqCst);
}

pub fn cancel_update() {
    CANCEL_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    CANCEL_REQUESTED.load(Ordering::SeqCst)
}

const OWNER: &str = "EasyTier";
const REPO: &str = "EasyTier";
pub const CURRENT_VERSION: &str = "2.6.4";

#[derive(Clone, Serialize, Deserialize)]
pub struct KernelUpdateInfo {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub asset_name: Option<String>,
    pub update_available: bool,
    pub available_versions: Option<Vec<String>>,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct KernelUpdateProgress {
    pub phase: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percent: Option<u8>,
    pub current_file: Option<String>,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub fn emit_progress(
    app: &AppHandle,
    phase: &str,
    message: impl Into<String>,
    downloaded: u64,
    total: Option<u64>,
    file: Option<String>,
    error: Option<String>,
) {
    let percent = total
        .filter(|x| *x > 0)
        .map(|x| ((downloaded.saturating_mul(100) / x).min(100)) as u8);
    let _ = app.emit(
        "kernel-update-progress",
        KernelUpdateProgress {
            phase: phase.into(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            percent,
            current_file: file,
            message: message.into(),
            error,
        },
    );
}

pub fn asset_for_target() -> Result<&'static str, String> {
    if cfg!(target_arch = "x86_64") {
        Ok("easytier-windows-x86_64")
    } else if cfg!(target_arch = "aarch64") {
        Ok("easytier-windows-arm64")
    } else {
        Err(format!("不支持的 Windows 架构：{}", std::env::consts::ARCH))
    }
}

fn release_version(tag: &str) -> String {
    tag.strip_prefix('v').unwrap_or(tag).to_string()
}

pub fn build_download_url(proxy: &str, raw: &str) -> Result<String, String> {
    let proxy = proxy.trim().trim_end_matches('/');
    if proxy.is_empty() || proxy == "direct" {
        return Ok(raw.to_string());
    }
    let parsed = reqwest::Url::parse(proxy).map_err(|e| format!("代理地址无效：{e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("代理地址必须使用 http 或 https".into());
    }
    Ok(format!("{proxy}/{raw}"))
}

fn fetch_release(client: &Client, proxy: &str) -> Result<Release, String> {
    let api = build_download_url(
        proxy,
        &format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/latest"),
    )?;
    client
        .get(api)
        .header("User-Agent", "easytier-win-client")
        .send()
        .map_err(|e| format!("请求 GitHub Release 失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub Release 响应错误：{e}"))?
        .json()
        .map_err(|e| format!("解析 Release 信息失败：{e}"))
}

pub fn fetch_release_by_tag(client: &Client, proxy: &str, tag: &str) -> Result<Release, String> {
    let tag = tag.trim();
    let tag_with_v = if tag.starts_with('v') {
        tag.to_string()
    } else {
        format!("v{tag}")
    };
    let api = build_download_url(
        proxy,
        &format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/tags/{tag_with_v}"),
    )?;
    let res = client
        .get(&api)
        .header("User-Agent", "easytier-win-client")
        .send()
        .map_err(|e| format!("请求指定版本 Release 失败：{e}"))?;
    if !res.status().is_success() && !tag.starts_with('v') {
        let api_raw = build_download_url(
            proxy,
            &format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/tags/{tag}"),
        )?;
        return client
            .get(api_raw)
            .header("User-Agent", "easytier-win-client")
            .send()
            .map_err(|e| format!("请求指定版本 Release 失败：{e}"))?
            .error_for_status()
            .map_err(|e| format!("GitHub Release 响应错误：{e}"))?
            .json()
            .map_err(|e| format!("解析 Release 信息失败：{e}"));
    }
    res.error_for_status()
        .map_err(|e| format!("GitHub Release 响应错误：{e}"))?
        .json()
        .map_err(|e| format!("解析 Release 信息失败：{e}"))
}

pub fn list_available_versions(proxy: &str) -> Result<Vec<String>, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let api = build_download_url(
        proxy,
        &format!("https://api.github.com/repos/{OWNER}/{REPO}/releases?per_page=25"),
    )?;
    let releases: Vec<Release> = client
        .get(api)
        .header("User-Agent", "easytier-win-client")
        .send()
        .map_err(|e| format!("获取 Release 列表失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub Release 列表响应错误：{e}"))?
        .json()
        .map_err(|e| format!("解析 Release 列表失败：{e}"))?;

    let stem = asset_for_target()?;
    let mut versions = Vec::new();
    for r in releases {
        if r.draft {
            continue;
        }
        let ver = release_version(&r.tag_name);
        let expected = format!("{stem}-v{ver}.zip");
        if r.assets.iter().any(|x| x.name == expected) {
            versions.push(r.tag_name);
        }
    }
    Ok(versions)
}

pub fn detect_current_version(core_path: Option<&Path>) -> String {
    if let Some(p) = core_path {
        if p.exists() {
            let mut cmd = std::process::Command::new(p);
            cmd.arg("--version")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            if let Ok(output) = cmd.output() {
                if let Ok(out_str) = String::from_utf8(output.stdout) {
                    for token in out_str.split_whitespace() {
                        let clean = token.trim().trim_start_matches('v');
                        if clean.chars().any(|c| c.is_ascii_digit()) && clean.contains('.') {
                            return clean.to_string();
                        }
                    }
                }
            }
        }
    }
    CURRENT_VERSION.to_string()
}

pub fn check(proxy: &str, core_path: Option<&Path>) -> Result<KernelUpdateInfo, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let release = fetch_release(&client, proxy)?;
    if release.draft || release.prerelease {
        return Err("GitHub 最新版本不是正式 Release".into());
    }
    let latest = release_version(&release.tag_name);
    let stem = asset_for_target()?;
    let expected = format!("{stem}-v{latest}.zip");
    let asset = release
        .assets
        .iter()
        .find(|x| x.name == expected)
        .ok_or_else(|| format!("Release 中没有当前架构资产：{expected}"))?;
    let current_ver = detect_current_version(core_path);
    let has_update = is_newer_version(&latest, &current_ver);
    let available_versions = list_available_versions(proxy).ok();
    Ok(KernelUpdateInfo {
        current_version: current_ver,
        latest_version: Some(latest),
        asset_name: Some(asset.name.clone()),
        update_available: has_update,
        available_versions,
        error: None,
    })
}

fn parse_version_tuple(v: &str) -> Option<(u32, u32, u32)> {
    let clean = v.trim().trim_start_matches('v');
    let base = clean.split('-').next().unwrap_or(clean);
    let mut parts = base.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    match (parse_version_tuple(latest), parse_version_tuple(current)) {
        (Some(l), Some(c)) => l > c,
        _ => {
            let l_base = latest.trim().trim_start_matches('v').split('-').next().unwrap_or("");
            let c_base = current.trim().trim_start_matches('v').split('-').next().unwrap_or("");
            !l_base.is_empty() && l_base != c_base
        }
    }
}

fn validate_archive(path: &Path, target: &Path) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|e| format!("打开下载包失败：{e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("读取 ZIP 失败：{e}"))?;
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let mut has_core = false;
    let mut has_cli = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let clean = name.trim_start_matches('/');
        if clean.contains("..") {
            return Err("ZIP 包含非法路径".into());
        }
        let base = Path::new(clean)
            .file_name()
            .and_then(|x| x.to_str())
            .unwrap_or(clean);
        if base == "easytier-core.exe" {
            has_core = true;
        }
        if base == "easytier-cli.exe" {
            has_cli = true;
        }
        let out = target.join(base);
        if entry.is_dir() {
            continue;
        }
        let mut output = fs::File::create(&out).map_err(|e| format!("写入 {base} 失败：{e}"))?;
        std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
    }
    if !has_core || !has_cli {
        return Err("ZIP 包缺少 easytier-core.exe 或 easytier-cli.exe".into());
    }
    Ok(())
}

pub fn download_and_stage(
    app: &AppHandle,
    proxy: &str,
    target_version: Option<&str>,
    runtime_dir: &Path,
) -> Result<PathBuf, String> {
    download_and_stage_with_progress(
        proxy,
        target_version,
        runtime_dir,
        |phase, message, downloaded, total, file, error| {
            emit_progress(app, phase, message, downloaded, total, file, error);
        },
    )
}

pub fn download_and_stage_with_progress<F>(
    proxy: &str,
    target_version: Option<&str>,
    runtime_dir: &Path,
    mut progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(&str, String, u64, Option<u64>, Option<String>, Option<String>),
{
    reset_cancel();
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    progress(
        "checking",
        "正在获取内核版本信息".into(),
        0,
        None,
        None,
        None,
    );
    if is_cancelled() {
        return Err("用户取消了内核更新".into());
    }
    let release = match target_version.filter(|s| !s.trim().is_empty()) {
        Some(tag) => fetch_release_by_tag(&client, proxy, tag)?,
        None => fetch_release(&client, proxy)?,
    };
    if release.draft {
        return Err("选定版本处于 Draft 草稿状态，无法下载".into());
    }
    if is_cancelled() {
        return Err("用户取消了内核更新".into());
    }
    let version = release_version(&release.tag_name);
    let stem = asset_for_target()?;
    let expected = format!("{stem}-v{version}.zip");
    let asset = release
        .assets
        .iter()
        .find(|x| x.name == expected)
        .ok_or_else(|| format!("Release 中没有当前架构资产：{expected}"))?;

    let parent = runtime_dir.parent().ok_or("无法确定 core 父目录")?;
    let temp = parent.join(format!(".easytier-update-{version}"));
    let staged = temp.join("core");
    let zip_path = temp.join(&asset.name);

    // 1. Check if we already have a fully extracted and verified core in cache!
    if staged.join("easytier-core.exe").exists() && staged.join("easytier-cli.exe").exists() {
        progress(
            "extracting",
            format!("发现已缓存的 v{version} 内核，直接复用…"),
            1,
            Some(1),
            Some(asset.name.clone()),
            None,
        );
        return Ok(staged);
    }

    // 2. Check if the downloaded zip archive exists and is valid
    if zip_path.exists() {
        progress(
            "extracting",
            format!("发现已下载的 v{version} 压缩包，正在校验解压…"),
            0,
            None,
            Some(asset.name.clone()),
            None,
        );
        if validate_archive(&zip_path, &staged).is_ok() {
            progress(
                "extracting",
                format!("v{version} 安装包校验完成，解压就绪"),
                1,
                Some(1),
                Some(asset.name.clone()),
                None,
            );
            return Ok(staged);
        }
    }

    // 3. Not cached or invalid: download from GitHub / proxy
    let url = build_download_url(proxy, &asset.browser_download_url)?;
    let mut response = client
        .get(url)
        .header("User-Agent", "easytier-win-client")
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total = response.content_length();
    if temp.exists() {
        let _ = fs::remove_dir_all(&temp);
    }
    fs::create_dir_all(&temp).map_err(|e| e.to_string())?;
    let mut output = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        if is_cancelled() {
            drop(output);
            let _ = fs::remove_dir_all(&temp);
            return Err("用户取消了内核下载".into());
        }
        let n = response.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        progress(
            "downloading",
            format!("已下载 {downloaded} 字节"),
            downloaded,
            total,
            Some(asset.name.clone()),
            None,
        );
    }
    drop(output);
    if is_cancelled() {
        let _ = fs::remove_dir_all(&temp);
        return Err("用户取消了内核下载".into());
    }
    progress(
        "extracting",
        "正在校验并解压内核".into(),
        downloaded,
        total,
        None,
        None,
    );
    validate_archive(&zip_path, &staged)?;
    Ok(staged)
}

fn cleanup_old_backups(parent: &Path, keep_count: usize) {
    if let Ok(entries) = fs::read_dir(parent) {
        let mut backups: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("core-backup-") {
                        backups.push(entry.path());
                    }
                }
            }
        }
        backups.sort();
        if backups.len() > keep_count {
            let to_remove = backups.len() - keep_count;
            for b in backups.iter().take(to_remove) {
                let _ = fs::remove_dir_all(b);
            }
        }
    }
}

pub fn install(runtime_dir: &Path, staged: &Path) -> Result<PathBuf, String> {
    let parent = runtime_dir.parent().ok_or("无法确定 core 父目录")?;
    let backup = parent.join(format!("core-backup-{}", chrono_like_timestamp()));

    // Create backup of current runtime_dir
    if runtime_dir.exists() {
        let _ = fs::create_dir_all(&backup);
        if let Ok(entries) = fs::read_dir(runtime_dir) {
            for entry in entries.flatten() {
                let _ = fs::copy(entry.path(), backup.join(entry.file_name()));
            }
        }
    }

    // Copy new staged files into runtime_dir (preserving staged cache for future instant switching!)
    if !runtime_dir.exists() {
        fs::create_dir_all(runtime_dir).map_err(|e| format!("创建 core 目录失败: {e}"))?;
    }
    let entries = fs::read_dir(staged).map_err(|e| format!("读取 staged 目录失败: {e}"))?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let dest = runtime_dir.join(&file_name);
        let mut copied = false;
        for _ in 0..5 {
            if fs::copy(entry.path(), &dest).is_ok() {
                copied = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        if !copied {
            // Restore from backup on failure
            if backup.exists() {
                if let Ok(b_entries) = fs::read_dir(&backup) {
                    for b_entry in b_entries.flatten() {
                        let _ = fs::copy(b_entry.path(), runtime_dir.join(b_entry.file_name()));
                    }
                }
            }
            return Err(format!("覆盖核心文件 {:?} 失败：旧进程可能仍在占用该文件", file_name));
        }
    }

    // Automatically prune old backup folders (keep only 1 latest)
    cleanup_old_backups(parent, 1);

    Ok(backup)
}

fn chrono_like_timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|x| x.as_secs())
        .unwrap_or(0)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asset_url_joins_proxy() {
        assert_eq!(
            build_download_url("https://ghfast.top/", "https://github.com/a.zip").unwrap(),
            "https://ghfast.top/https://github.com/a.zip"
        );
    }
    #[test]
    fn direct_url_is_unchanged() {
        assert_eq!(
            build_download_url("direct", "https://github.com/a.zip").unwrap(),
            "https://github.com/a.zip"
        );
    }
    #[test]
    fn version_strips_v() {
        assert_eq!(release_version("v2.6.4"), "2.6.4");
    }
    #[test]
    fn version_comparison_handles_git_commit_hash() {
        assert!(!is_newer_version("2.6.4", "2.6.4-8428a89d"));
        assert!(!is_newer_version("v2.6.4", "2.6.4-8428a89d"));
        assert!(!is_newer_version("2.6.4", "2.6.4"));
        assert!(is_newer_version("2.6.5", "2.6.4-8428a89d"));
        assert!(is_newer_version("2.7.0", "2.6.4-8428a89d"));
        assert!(!is_newer_version("2.6.3", "2.6.4-8428a89d"));
    }
}
