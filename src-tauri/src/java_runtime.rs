use crate::settings::{inspect_java, JavaInfo};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tokio::io::AsyncWriteExt;

static JAVA_INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Deserialize)]
struct Release {
    binary: Binary,
}
#[derive(Deserialize)]
struct Binary {
    package: Package,
}
#[derive(Deserialize)]
struct Package {
    checksum: String,
    link: String,
    name: String,
    size: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JavaProgress {
    major: u32,
    downloaded: u64,
    total: u64,
    message: String,
}

fn emit(
    app: &tauri::AppHandle,
    major: u32,
    downloaded: u64,
    total: u64,
    message: impl Into<String>,
) {
    let _ = app.emit(
        "java-progress",
        JavaProgress {
            major,
            downloaded,
            total,
            message: message.into(),
        },
    );
}

fn required_major(root: &Path, version: &str) -> Result<u32, String> {
    if version.is_empty() || version.len() > 100 || version.contains(['/', '\\', ':']) {
        return Err("请先选择有效的游戏版本".into());
    }
    let metadata = root
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    let json: serde_json::Value = serde_json::from_slice(
        &fs::read(metadata).map_err(|_| "请先下载游戏版本，再自动配置对应 Java".to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(json["javaVersion"]["majorVersion"].as_u64().unwrap_or(8) as u32)
}

fn find_java(root: &Path) -> Option<PathBuf> {
    let mut pending = vec![(root.to_owned(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > 3 {
            continue;
        }
        for entry in fs::read_dir(directory).ok()?.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("java.exe"))
                && path
                    .parent()?
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
            {
                return Some(path);
            }
            if entry.file_type().ok()?.is_dir() {
                pending.push((path, depth + 1));
            }
        }
    }
    None
}

fn extract(archive: &Path, destination: &Path) -> Result<PathBuf, String> {
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Java 压缩包包含不安全的符号链接".into());
        }
        let target = destination.join(entry.enclosed_name().ok_or("Java 压缩包包含不安全路径")?);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else {
            fs::create_dir_all(target.parent().ok_or("无效解压路径")?)
                .map_err(|e| e.to_string())?;
            let mut output = fs::File::create(target).map_err(|e| e.to_string())?;
            let mut buffer = [0u8; 65_536];
            loop {
                let count = entry.read(&mut buffer).map_err(|e| e.to_string())?;
                if count == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    find_java(destination).ok_or_else(|| "下载的 Java 中没有找到 bin/java.exe".into())
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            let _ = fs::remove_file(&self.0);
        }
    }
}

#[tauri::command]
pub async fn install_java(
    app: tauri::AppHandle,
    game_dir: String,
    version: String,
) -> Result<JavaInfo, String> {
    let _guard = JAVA_INSTALL.lock().await;
    let game_root = PathBuf::from(game_dir);
    if !game_root.is_absolute() {
        return Err("游戏目录必须是绝对路径".into());
    }
    let major = tokio::task::spawn_blocking(move || required_major(&game_root, &version))
        .await
        .map_err(|e| e.to_string())??;
    let runtime_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("runtimes");
    tokio::fs::create_dir_all(&runtime_root)
        .await
        .map_err(|e| e.to_string())?;
    let mut entries = tokio::fs::read_dir(&runtime_root)
        .await
        .map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(&format!("temurin-{major}-"))
        {
            if let Some(java) = find_java(&entry.path()) {
                if let Ok(info) = inspect_java(java.to_string_lossy().into_owned()).await {
                    if info.major == major {
                        return Ok(info);
                    }
                }
            }
        }
    }
    emit(&app, major, 0, 0, format!("查询 Temurin Java {major}…"));
    let url = format!("https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse&heap_size=normal");
    let http = reqwest::Client::builder()
        .https_only(true)
        .user_agent("CatCL/0.1.0")
        .build()
        .map_err(|e| e.to_string())?;
    let releases: Vec<Release> = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("无法连接 Adoptium：{e}"))?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let package = releases
        .into_iter()
        .next()
        .ok_or_else(|| format!("没有找到 Windows x64 Temurin Java {major}"))?
        .binary
        .package;
    if package.checksum.len() != 64 || !package.checksum.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Java 下载校验值格式错误".into());
    }
    let key = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let archive = Cleanup(runtime_root.join(format!(".java-{key}.zip")));
    let stage = Cleanup(runtime_root.join(format!(".java-{key}")));
    let response = http
        .get(&package.link)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total = response.content_length().unwrap_or(package.size);
    let mut chunks = response.bytes_stream();
    let mut output = tokio::fs::File::create(&archive.0)
        .await
        .map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut downloaded = 0;
    while let Some(chunk) = chunks.next().await {
        let bytes = chunk.map_err(|e| e.to_string())?;
        digest.update(&bytes);
        output.write_all(&bytes).await.map_err(|e| e.to_string())?;
        downloaded += bytes.len() as u64;
        emit(
            &app,
            major,
            downloaded,
            total,
            format!("正在下载 {}", package.name),
        );
    }
    output.flush().await.map_err(|e| e.to_string())?;
    drop(output);
    if !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(&package.checksum) {
        return Err("Java 下载 SHA-256 校验失败".into());
    }
    emit(&app, major, downloaded, total, "正在解压 Java…");
    let archive_path = archive.0.clone();
    let stage_path = stage.0.clone();
    let java = tokio::task::spawn_blocking(move || extract(&archive_path, &stage_path))
        .await
        .map_err(|e| e.to_string())??;
    let relative = java
        .strip_prefix(&stage.0)
        .map_err(|e| e.to_string())?
        .to_owned();
    let final_dir = runtime_root.join(format!("temurin-{major}-{}-{key}", &package.checksum[..12]));
    tokio::fs::rename(&stage.0, &final_dir)
        .await
        .map_err(|e| e.to_string())?;
    let info = inspect_java(final_dir.join(relative).to_string_lossy().into_owned()).await?;
    emit(&app, major, total, total, format!("Java {major} 已安装"));
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_required_java_from_metadata() {
        let root = std::env::temp_dir().join(format!("catcl-java-test-{}", std::process::id()));
        fs::create_dir_all(root.join("versions/1.21.1")).unwrap();
        fs::write(
            root.join("versions/1.21.1/1.21.1.json"),
            br#"{"javaVersion":{"majorVersion":21}}"#,
        )
        .unwrap();
        assert_eq!(required_major(&root, "1.21.1").unwrap(), 21);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires access to Eclipse Adoptium"]
    async fn official_api_exposes_checksum_for_windows_x64_jre() {
        let releases: Vec<Release> = reqwest::Client::new()
            .get("https://api.adoptium.net/v3/assets/latest/21/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse&heap_size=normal")
            .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        let package = &releases.first().unwrap().binary.package;
        assert_eq!(package.checksum.len(), 64);
        assert!(package.link.starts_with("https://"));
        assert!(package.name.ends_with(".zip"));
    }
}
