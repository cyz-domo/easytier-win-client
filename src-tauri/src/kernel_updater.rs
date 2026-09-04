use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}, time::Duration};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

const OWNER: &str = "EasyTier";
const REPO: &str = "EasyTier";
pub const CURRENT_VERSION: &str = "2.6.4";

#[derive(Clone, Serialize, Deserialize)]
pub struct KernelUpdateInfo {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub asset_name: Option<String>,
    pub update_available: bool,
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

pub fn emit_progress(app: &AppHandle, phase: &str, message: impl Into<String>, downloaded: u64, total: Option<u64>, file: Option<String>, error: Option<String>) {
    let percent = total.filter(|x| *x > 0).map(|x| ((downloaded.saturating_mul(100) / x).min(100)) as u8);
    let _ = app.emit("kernel-update-progress", KernelUpdateProgress {
        phase: phase.into(), downloaded_bytes: downloaded, total_bytes: total, percent,
        current_file: file, message: message.into(), error,
    });
}

pub fn asset_for_target() -> Result<&'static str, String> {
    if cfg!(target_arch = "x86_64") { Ok("easytier-windows-x86_64") }
    else if cfg!(target_arch = "aarch64") { Ok("easytier-windows-arm64") }
    else { Err(format!("不支持的 Windows 架构：{}", std::env::consts::ARCH)) }
}

fn release_version(tag: &str) -> String { tag.strip_prefix('v').unwrap_or(tag).to_string() }

pub fn build_download_url(proxy: &str, raw: &str) -> Result<String, String> {
    let proxy = proxy.trim().trim_end_matches('/');
    if proxy.is_empty() || proxy == "direct" { return Ok(raw.to_string()); }
    let parsed = reqwest::Url::parse(proxy).map_err(|e| format!("代理地址无效：{e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" { return Err("代理地址必须使用 http 或 https".into()); }
    Ok(format!("{proxy}/{raw}"))
}

fn fetch_release(client: &Client, proxy: &str) -> Result<Release, String> {
    let api = build_download_url(proxy, &format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/latest"))?;
    client.get(api)
        .header("User-Agent", "easytier-win-client")
        .send().map_err(|e| format!("请求 GitHub Release 失败：{e}"))?
        .error_for_status().map_err(|e| format!("GitHub Release 响应错误：{e}"))?
        .json().map_err(|e| format!("解析 Release 信息失败：{e}"))
}

pub fn check(proxy: &str) -> Result<KernelUpdateInfo, String> {
    let client = Client::builder().timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let release = fetch_release(&client, proxy)?;
    if release.draft || release.prerelease { return Err("GitHub 最新版本不是正式 Release".into()); }
    let latest = release_version(&release.tag_name);
    let stem = asset_for_target()?;
    let expected = format!("{stem}-v{latest}.zip");
    let asset = release.assets.iter().find(|x| x.name == expected).ok_or_else(|| format!("Release 中没有当前架构资产：{expected}"))?;
    Ok(KernelUpdateInfo { current_version: CURRENT_VERSION.into(), latest_version: Some(latest.clone()), asset_name: Some(asset.name.clone()), update_available: latest != CURRENT_VERSION, error: None })
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
        if clean.contains("..") { return Err("ZIP 包含非法路径".into()); }
        let base = Path::new(clean).file_name().and_then(|x| x.to_str()).unwrap_or(clean);
        if base == "easytier-core.exe" { has_core = true; }
        if base == "easytier-cli.exe" { has_cli = true; }
        let out = target.join(base);
        if entry.is_dir() { continue; }
        let mut output = fs::File::create(&out).map_err(|e| format!("写入 {base} 失败：{e}"))?;
        std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
    }
    if !has_core || !has_cli { return Err("ZIP 包缺少 easytier-core.exe 或 easytier-cli.exe".into()); }
    Ok(())
}

pub fn download_and_stage(app: &AppHandle, proxy: &str, runtime_dir: &Path) -> Result<PathBuf, String> {
    let client = Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    emit_progress(app, "checking", "正在获取最新内核信息", 0, None, None, None);
    let release = fetch_release(&client, proxy)?;
    if release.draft || release.prerelease { return Err("GitHub 最新版本不是正式 Release".into()); }
    let version = release_version(&release.tag_name);
    let stem = asset_for_target()?;
    let expected = format!("{stem}-v{version}.zip");
    let asset = release.assets.iter().find(|x| x.name == expected).ok_or_else(|| format!("Release 中没有当前架构资产：{expected}"))?;
    let url = build_download_url(proxy, &asset.browser_download_url)?;
    let mut response = client.get(url).header("User-Agent", "easytier-win-client").send().map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?;
    let total = response.content_length();
    let parent = runtime_dir.parent().ok_or("无法确定 core 父目录")?;
    let temp = parent.join(format!(".easytier-update-{version}"));
    if temp.exists() { fs::remove_dir_all(&temp).map_err(|e| e.to_string())?; }
    fs::create_dir_all(&temp).map_err(|e| e.to_string())?;
    let zip_path = temp.join(&asset.name);
    let mut output = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        let n = response.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        output.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        emit_progress(app, "downloading", format!("已下载 {downloaded} 字节"), downloaded, total, Some(asset.name.clone()), None);
    }
    drop(output);
    let staged = temp.join("core");
    emit_progress(app, "extracting", "正在校验并解压内核", downloaded, total, None, None);
    validate_archive(&zip_path, &staged)?;
    Ok(staged)
}

pub fn install(runtime_dir: &Path, staged: &Path) -> Result<PathBuf, String> {
    let parent = runtime_dir.parent().ok_or("无法确定 core 父目录")?;
    let backup = parent.join(format!("core-backup-{}", chrono_like_timestamp()));
    if runtime_dir.exists() { fs::rename(runtime_dir, &backup).map_err(|e| format!("备份旧 core 失败：{e}"))?; }
    match fs::rename(staged, runtime_dir) {
        Ok(()) => Ok(backup),
        Err(e) => {
            if backup.exists() { let _ = fs::rename(&backup, runtime_dir); }
            Err(format!("安装新 core 失败：{e}"))
        }
    }
}

fn chrono_like_timestamp() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|x| x.as_secs()).unwrap_or(0).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn asset_url_joins_proxy() { assert_eq!(build_download_url("https://ghfast.top/", "https://github.com/a.zip").unwrap(), "https://ghfast.top/https://github.com/a.zip"); }
    #[test] fn direct_url_is_unchanged() { assert_eq!(build_download_url("direct", "https://github.com/a.zip").unwrap(), "https://github.com/a.zip"); }
    #[test] fn version_strips_v() { assert_eq!(release_version("v2.6.4"), "2.6.4"); }
}
