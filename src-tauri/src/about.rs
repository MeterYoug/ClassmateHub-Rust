use crate::{database, recovery::Storage, seating};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Write, path::PathBuf, time::Duration};
use tauri::{AppHandle, Manager, WebviewWindow};

const REPO_URL: &str = "https://github.com/MeterYoug/ClassmateHub-Releases";
const RELEASES_URL: &str = "https://github.com/MeterYoug/ClassmateHub-Releases/releases";
const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/MeterYoug/ClassmateHub-Releases/releases/latest/download/latest.json";
const DEFAULT_RELEASE_API_URL: &str =
    "https://api.github.com/repos/MeterYoug/ClassmateHub-Releases/releases/latest";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_ASSET_BYTES: usize = 600 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct AboutInfo {
    pub app_name: &'static str,
    pub product_title: &'static str,
    pub version: &'static str,
    pub identifier: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub sqlite_schema_version: i64,
    pub engine_version: u32,
    pub license: &'static str,
    pub repository_url: &'static str,
    pub releases_url: &'static str,
    pub default_manifest_url: &'static str,
    pub data_directory: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub has_update: bool,
    pub status: String,
    pub message: String,
    pub release_title: Option<String>,
    pub notes: Option<String>,
    pub published_at: Option<String>,
    pub release_page_url: String,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
    pub asset_size: Option<u64>,
    pub source_endpoint: String,
}

pub fn collect_about_info(data_directory: String) -> AboutInfo {
    AboutInfo {
        app_name: "班伴",
        product_title: "班伴 ClassmateHub",
        version: env!("CARGO_PKG_VERSION"),
        identifier: "com.classmatehub.desktop",
        os: current_os_label(),
        arch: std::env::consts::ARCH,
        sqlite_schema_version: database::VERSION,
        engine_version: seating::ENGINE_VERSION,
        license: "MIT",
        repository_url: REPO_URL,
        releases_url: RELEASES_URL,
        default_manifest_url: DEFAULT_MANIFEST_URL,
        data_directory,
    }
}

fn current_os_label() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

pub fn normalize_version(raw: &str) -> Option<(u64, u64, u64, String)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    if trimmed.is_empty() {
        return None;
    }
    let (core, suffix) = match trimmed.find(['-', '+']) {
        Some(idx) => (&trimmed[..idx], trimmed[idx..].to_string()),
        None => (trimmed, String::new()),
    };
    let mut parts = core.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next().unwrap_or("0").parse().ok()?;
    let patch: u64 = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, suffix))
}

pub fn is_newer_version(current: &str, candidate: &str) -> Result<bool, String> {
    let (c_maj, c_min, c_pat, c_suf) =
        normalize_version(current).ok_or_else(|| format!("当前版本号格式无效：{current}"))?;
    let (n_maj, n_min, n_pat, n_suf) =
        normalize_version(candidate).ok_or_else(|| format!("远端版本号格式无效：{candidate}"))?;
    let core_cmp = (n_maj, n_min, n_pat).cmp(&(c_maj, c_min, c_pat));
    Ok(match core_cmp {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => !c_suf.is_empty() && n_suf.is_empty(),
    })
}

pub fn validate_update_url(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    let url = Url::parse(trimmed).map_err(|_| "更新检查地址格式无效".to_string())?;
    let loopback = url
        .host_str()
        .map(|h| h.trim_matches(['[', ']']))
        .is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        });
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 512
    {
        return Err("更新检查地址须为完整的 HTTPS 地址（本机回环测试可使用 HTTP）".into());
    }
    Ok(url)
}

fn current_platform_keys() -> &'static [&'static str] {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => &["windows-x86_64", "windows-x86_64-nsis", "win64"],
        ("macos", "aarch64") => &["darwin-aarch64", "macos-aarch64", "darwin-arm64"],
        ("macos", "x86_64") => &["darwin-x86_64", "macos-x86_64"],
        ("linux", "x86_64") => &["linux-x86_64", "linux-x86_64-deb"],
        _ => &[],
    }
}

fn matches_platform_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".txt") || lower.ends_with(".sig") || lower.ends_with(".zip") {
        return false;
    }
    match std::env::consts::OS {
        "windows" => lower.ends_with("-setup.exe") || lower.ends_with(".exe") || lower.ends_with(".msi"),
        "macos" => lower.ends_with(".dmg") || lower.ends_with(".app.tar.gz"),
        "linux" => lower.ends_with(".deb") || lower.ends_with(".appimage"),
        _ => false,
    }
}

pub fn parse_update_payload(
    current_version: &str,
    source_endpoint: &str,
    payload: &Value,
) -> Result<UpdateCheckResult, String> {
    let object = payload.as_object().ok_or("更新清单不是有效的 JSON 对象")?;

    // 1. GitHub Releases API 格式（含 tag_name）
    if let Some(tag_name) = object.get("tag_name").and_then(Value::as_str) {
        let clean_latest = tag_name.trim().trim_start_matches(['v', 'V']).to_string();
        let has_update = is_newer_version(current_version, &clean_latest)?;
        let release_title = object
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| Some(format!("v{clean_latest}")));
        let notes = object
            .get("body")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let published_at = object
            .get("published_at")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let release_page_url = object
            .get("html_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
            .unwrap_or(RELEASES_URL)
            .to_string();

        let mut download_url = None;
        let mut asset_name = None;
        let mut asset_size = None;
        if let Some(assets) = object.get("assets").and_then(Value::as_array) {
            if let Some(matched) = assets.iter().find(|item| {
                item.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(matches_platform_asset)
            }) {
                asset_name = matched.get("name").and_then(Value::as_str).map(String::from);
                asset_size = matched.get("size").and_then(Value::as_u64);
                download_url = matched
                    .get("browser_download_url")
                    .and_then(Value::as_str)
                    .map(String::from);
            }
        }

        let message = if has_update {
            format!("发现新版本 v{clean_latest}（当前版本 v{current_version}）")
        } else {
            format!("当前版本 v{current_version} 已是最新版本")
        };
        return Ok(UpdateCheckResult {
            current_version: current_version.to_string(),
            latest_version: Some(clean_latest),
            has_update,
            status: if has_update { "update_available".into() } else { "up_to_date".into() },
            message,
            release_title,
            notes,
            published_at,
            release_page_url,
            download_url,
            asset_name,
            asset_size,
            source_endpoint: source_endpoint.to_string(),
        });
    }

    // 2. Tauri latest.json 清单格式（含 version）
    if let Some(version_str) = object.get("version").and_then(Value::as_str) {
        let clean_latest = version_str.trim().trim_start_matches(['v', 'V']).to_string();
        let has_update = is_newer_version(current_version, &clean_latest)?;
        let notes = object
            .get("notes")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let published_at = object
            .get("pub_date")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        let mut download_url = object
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let mut asset_name = None;

        if let Some(platforms) = object.get("platforms").and_then(Value::as_object) {
            for &key in current_platform_keys() {
                if let Some(entry) = platforms.get(key) {
                    if let Some(url) = entry.get("url").and_then(Value::as_str) {
                        download_url = Some(url.to_string());
                        asset_name = url.rsplit('/').next().map(String::from);
                        break;
                    }
                }
            }
        }
        if asset_name.is_none() {
            asset_name = download_url
                .as_deref()
                .and_then(|u| u.rsplit('/').next())
                .filter(|s| !s.is_empty())
                .map(String::from);
        }

        let message = if has_update {
            format!("发现新版本 v{clean_latest}（当前版本 v{current_version}）")
        } else {
            format!("当前版本 v{current_version} 已是最新版本")
        };
        let release_title = Some(format!("班伴 v{clean_latest}"));
        return Ok(UpdateCheckResult {
            current_version: current_version.to_string(),
            latest_version: Some(clean_latest),
            has_update,
            status: if has_update { "update_available".into() } else { "up_to_date".into() },
            message,
            release_title,
            notes,
            published_at,
            release_page_url: RELEASES_URL.to_string(),
            download_url,
            asset_name,
            asset_size: None,
            source_endpoint: source_endpoint.to_string(),
        });
    }

    Err("更新清单缺少 version 或 tag_name 字段".into())
}

async fn fetch_json(client: &Client, url: &Url) -> Result<Result<Value, StatusCode>, String> {
    let mut response = client
        .get(url.clone())
        .header("Accept", "application/vnd.github+json, application/json")
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "连接 GitHub Releases 超时，请检查网络或直接访问发布页".to_string()
            } else {
                "无法连接更新服务，请检查网络或直接访问 GitHub 发布页".to_string()
            }
        })?;

    if !response.status().is_success() {
        return Ok(Err(response.status()));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取更新清单响应失败".to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("更新清单响应超出大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }

    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "更新服务返回了无效的 JSON 数据".to_string())?;
    Ok(Ok(value))
}

pub async fn check_update_from_endpoints(
    current_version: &str,
    custom_endpoint: Option<String>,
) -> Result<UpdateCheckResult, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("ClassmateHub/{current_version}"))
        .build()
        .map_err(|_| "无法初始化更新检查客户端".to_string())?;

    if let Some(custom) = custom_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let url = validate_update_url(custom)?;
        match fetch_json(&client, &url).await? {
            Ok(payload) => return parse_update_payload(current_version, url.as_str(), &payload),
            Err(StatusCode::NOT_FOUND) => {
                return Ok(UpdateCheckResult {
                    current_version: current_version.to_string(),
                    latest_version: None,
                    has_update: false,
                    status: "no_release".into(),
                    message: "所选更新地址尚未发布可用版本（HTTP 404）".into(),
                    release_title: None,
                    notes: None,
                    published_at: None,
                    release_page_url: RELEASES_URL.to_string(),
                    download_url: None,
                    asset_name: None,
                    asset_size: None,
                    source_endpoint: url.to_string(),
                });
            }
            Err(code) => {
                return Err(format!("更新地址返回 HTTP {}", code.as_u16()));
            }
        }
    }

    // 默认路径：先查 GitHub Releases 下的 latest.json，若 404 再查 GitHub Releases latest API
    let manifest_url = validate_update_url(DEFAULT_MANIFEST_URL)?;
    if let Ok(Ok(payload)) = fetch_json(&client, &manifest_url).await {
        if let Ok(result) = parse_update_payload(current_version, manifest_url.as_str(), &payload) {
            return Ok(result);
        }
    }

    let api_url = validate_update_url(DEFAULT_RELEASE_API_URL)?;
    match fetch_json(&client, &api_url).await? {
        Ok(payload) => parse_update_payload(current_version, api_url.as_str(), &payload),
        Err(StatusCode::NOT_FOUND) => Ok(UpdateCheckResult {
            current_version: current_version.to_string(),
            latest_version: None,
            has_update: false,
            status: "no_release".into(),
            message: format!(
                "GitHub 仓库暂未发布公开 Release，当前版本为 v{current_version}"
            ),
            release_title: None,
            notes: None,
            published_at: None,
            release_page_url: RELEASES_URL.to_string(),
            download_url: None,
            asset_name: None,
            asset_size: None,
            source_endpoint: api_url.to_string(),
        }),
        Err(StatusCode::FORBIDDEN) | Err(StatusCode::TOO_MANY_REQUESTS) => Err(
            "GitHub 接口访问受限或触发频率限制，请稍后重试或直接打开 GitHub Releases 页面".into(),
        ),
        Err(code) => Err(format!("GitHub Releases 返回 HTTP {}", code.as_u16())),
    }
}

#[tauri::command]
pub fn about_info<R: tauri::Runtime>(app: AppHandle<R>) -> AboutInfo {
    let data_dir = app
        .try_state::<Storage>()
        .map(|storage| storage.data_dir().display().to_string())
        .or_else(|| {
            app.path()
                .app_data_dir()
                .ok()
                .map(|path| path.display().to_string())
        })
        .unwrap_or_else(|| "未配置应用数据目录".into());
    collect_about_info(data_dir)
}

#[tauri::command]
pub async fn check_app_update(endpoint: Option<String>) -> Result<UpdateCheckResult, String> {
    check_update_from_endpoints(env!("CARGO_PKG_VERSION"), endpoint).await
}

fn sanitize_filename(raw: Option<&str>) -> String {
    let fallback = match std::env::consts::OS {
        "windows" => "班伴_latest_x64-setup.exe",
        "macos" => "班伴_latest_aarch64.dmg",
        "linux" => "classmatehub_latest_amd64.deb",
        _ => "classmatehub-update.bin",
    };
    let candidate = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.rsplit(['/', '\\']).next())
        .unwrap_or(fallback);
    let cleaned: String = candidate
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

pub async fn download_asset_to_path(url: &Url, target: &PathBuf) -> Result<(), String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(format!("ClassmateHub/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法初始化更新下载客户端".to_string())?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "下载安装包超时，请检查网络或使用浏览器从发布页下载".to_string()
            } else {
                "无法连接安装包下载地址，请检查网络或使用浏览器下载".to_string()
            }
        })?;
    if !response.status().is_success() {
        return Err(format!("下载安装包失败（HTTP {}）", response.status().as_u16()));
    }
    let parent = target.parent().ok_or("保存目录无效")?;
    let write_error = |e: std::io::Error| format!("写入安装包文件失败：{e}");
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(write_error)?;
    let mut total = 0usize;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "下载安装包过程中连接中断，请重试或通过发布页下载".to_string())?
    {
        total = total.saturating_add(chunk.len());
        if total > MAX_ASSET_BYTES {
            return Err("安装包大小超出安全限制（600 MB）".into());
        }
        temporary.write_all(&chunk).map_err(write_error)?;
    }
    if total == 0 {
        return Err("下载的安装包内容为空".into());
    }
    temporary.as_file().sync_all().map_err(write_error)?;
    temporary.persist(target).map_err(|e| write_error(e.error))?;
    Ok(())
}

#[tauri::command]
pub async fn download_update_asset<R: tauri::Runtime>(
    window: WebviewWindow<R>,
    url: String,
    suggested_name: Option<String>,
) -> Result<Option<String>, String> {
    let checked_url = validate_update_url(&url)?;
    let filename = sanitize_filename(suggested_name.as_deref());
    let chosen = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("保存新版班伴安装包")
            .set_file_name(&filename)
            .save_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(path) = chosen else {
        return Ok(None);
    };
    download_asset_to_path(&checked_url, &path).await?;
    Ok(Some(path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_semantic_versions_and_validates_endpoints() {
        assert!(is_newer_version("0.1.7", "0.1.8").unwrap());
        assert!(is_newer_version("0.1.8", "v0.2.0").unwrap());
        assert!(!is_newer_version("0.1.8", "0.1.8").unwrap());
        assert!(!is_newer_version("0.1.8", "0.1.7").unwrap());
        assert!(is_newer_version("0.1.8-beta.1", "0.1.8").unwrap());

        assert!(validate_update_url(DEFAULT_MANIFEST_URL).is_ok());
        assert!(validate_update_url("http://127.0.0.1:18080/latest.json").is_ok());
        assert!(validate_update_url("http://example.com/latest.json").is_err());
        assert!(validate_update_url("https://user:pass@example.com/latest.json").is_err());
    }

    #[test]
    fn parses_tauri_latest_json_and_github_release_payloads() {
        let tauri_manifest = json!({
            "version": "v0.2.0",
            "notes": "1. 新增关于与在线更新\n2. 优化排座空位",
            "pub_date": "2026-11-20T08:30:00Z",
            "platforms": {
                "windows-x86_64": {
                    "url": format!("{RELEASES_URL}/download/v0.2.0/ClassmateHub_0.2.0_windows-x86_64-setup.exe")
                },
                "darwin-aarch64": {
                    "url": format!("{RELEASES_URL}/download/v0.2.0/ClassmateHub_0.2.0_darwin-aarch64.dmg")
                },
                "linux-x86_64": {
                    "url": format!("{RELEASES_URL}/download/v0.2.0/ClassmateHub_0.2.0_linux-x86_64.deb")
                }
            }
        });
        let parsed =
            parse_update_payload("0.1.8", DEFAULT_MANIFEST_URL, &tauri_manifest).unwrap();
        assert!(parsed.has_update);
        assert_eq!(parsed.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(parsed.status, "update_available");
        assert_eq!(
            parsed.notes.as_deref(),
            Some("1. 新增关于与在线更新\n2. 优化排座空位")
        );
        let expected_asset = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Some("ClassmateHub_0.2.0_windows-x86_64-setup.exe"),
            ("macos", "aarch64") => Some("ClassmateHub_0.2.0_darwin-aarch64.dmg"),
            ("linux", "x86_64") => Some("ClassmateHub_0.2.0_linux-x86_64.deb"),
            _ => None,
        };
        assert_eq!(parsed.asset_name.as_deref(), expected_asset);

        let gh_release = json!({
            "tag_name": "v0.1.8",
            "name": "班伴 0.1.8",
            "body": "正式测试版",
            "published_at": "2026-11-20T08:30:00Z",
            "html_url": "https://github.com/MeterYoug/ClassmateHub-Rust/releases/tag/v0.1.8",
            "assets": [
                {
                    "name": "班伴_0.1.8_x64-setup.exe",
                    "size": 302526374,
                    "browser_download_url": "https://github.com/MeterYoug/ClassmateHub-Rust/releases/download/v0.1.8/班伴_0.1.8_x64-setup.exe"
                }
            ]
        });
        let up_to_date =
            parse_update_payload("0.1.8", DEFAULT_RELEASE_API_URL, &gh_release).unwrap();
        assert!(!up_to_date.has_update);
        assert_eq!(up_to_date.status, "up_to_date");
        assert_eq!(
            up_to_date.release_page_url,
            "https://github.com/MeterYoug/ClassmateHub-Rust/releases/tag/v0.1.8"
        );
    }

    #[tokio::test]
    async fn loopback_update_check_and_asset_download_work_end_to_end() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                if req.starts_with("GET /latest.json ") {
                    let body = format!(
                        r#"{{"version":"0.2.0","notes":"测试更新说明","url":"http://{addr}/班伴_0.2.0_x64-setup.exe"}}"#
                    );
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(resp.as_bytes()).unwrap();
                } else {
                    let payload = b"FAKE_INSTALLER_BINARY";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    stream.write_all(resp.as_bytes()).unwrap();
                    stream.write_all(payload).unwrap();
                }
            }
        });

        let endpoint = format!("http://{addr}/latest.json");
        let check = check_update_from_endpoints("0.1.8", Some(endpoint)).await.unwrap();
        assert!(check.has_update);
        assert_eq!(check.latest_version.as_deref(), Some("0.2.0"));
        let dl_url = validate_update_url(check.download_url.as_deref().unwrap()).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("班伴_0.2.0_x64-setup.exe");
        download_asset_to_path(&dl_url, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"FAKE_INSTALLER_BINARY");
        server.join().unwrap();
    }
}
