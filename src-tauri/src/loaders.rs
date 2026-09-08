use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

static LOADER_INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderInstallResult {
    loader: String,
    loader_version: String,
    launch_version: String,
}

fn xml_versions(xml: &str) -> Vec<String> {
    xml.split("<version>")
        .skip(1)
        .filter_map(|part| part.split("</version>").next())
        .map(str::to_owned)
        .collect()
}

#[tauri::command]
pub async fn list_loader_versions(
    game_version: String,
    loader: String,
) -> Result<Vec<String>, String> {
    let versions: Vec<String> = match loader.as_str() {
        "fabric" => {
            let value: serde_json::Value = reqwest::get(format!(
                "https://meta.fabricmc.net/v2/versions/loader/{game_version}"
            ))
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item["loader"]["version"].as_str().map(str::to_owned))
                .collect()
        }
        "forge" => {
            let xml = reqwest::get(
                "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml",
            )
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
            let prefix = format!("{game_version}-");
            xml_versions(&xml)
                .into_iter()
                .rev()
                .filter_map(|version| version.strip_prefix(&prefix).map(str::to_owned))
                .collect()
        }
        "neoforge" => {
            let mut parts = game_version.trim_start_matches("1.").split('.');
            let prefix = format!(
                "{}.{}.",
                parts.next().ok_or("无效的 Minecraft 版本")?,
                parts.next().unwrap_or("0")
            );
            let xml = reqwest::get(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml",
            )
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())?;
            xml_versions(&xml)
                .into_iter()
                .rev()
                .filter(|version| version.starts_with(&prefix))
                .collect()
        }
        _ => return Err("不支持的模组加载器".into()),
    };
    if versions.is_empty() {
        return Err("该加载器没有兼容当前 Minecraft 的版本".into());
    }
    Ok(versions)
}

async fn download(url: &str, path: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(20))
        .user_agent("CatCL/0.1.0")
        .build()
        .map_err(|e| e.to_string())?;
    let expected = match client.get(format!("{url}.sha1")).send().await {
        Ok(response) => match response.error_for_status() {
            Ok(response) => response.text().await.ok().map(|value| {
                value
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            }),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let temp = path.with_extension("jar.part");
    let mut last_error = String::new();
    for attempt in 0..3 {
        let result = async {
            let bytes = client
                .get(url)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .bytes()
                .await
                .map_err(|e| e.to_string())?;
            if let Some(expected) = &expected {
                use sha1::Digest;
                let actual = format!("{:x}", sha1::Sha1::digest(&bytes));
                if !actual.eq_ignore_ascii_case(expected) {
                    return Err("安装器 SHA-1 校验失败".into());
                }
            }
            tokio::fs::write(&temp, bytes)
                .await
                .map_err(|e| e.to_string())?;
            tokio::fs::rename(&temp, path)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = error;
                let _ = tokio::fs::remove_file(&temp).await;
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(700 * (attempt + 1))).await;
                }
            }
        }
    }
    Err(format!("加载器安装器下载失败：{last_error}"))
}

async fn run_installer(java: &str, jar: &Path, args: &[String], root: &Path) -> Result<(), String> {
    let mut command = tokio::process::Command::new(java.trim());
    command
        .arg("-jar")
        .arg(jar)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .await
        .map_err(|e| format!("无法运行模组加载器安装器：{e}"))?;
    if !output.status.success() {
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return Err(format!("模组加载器安装失败：{}", message.trim()));
    }
    Ok(())
}

async fn fabric(
    game: &str,
    loader: &str,
    root: &Path,
    java: &str,
    temp: &Path,
) -> Result<LoaderInstallResult, String> {
    let http = reqwest::Client::new();
    let installers: serde_json::Value = http
        .get("https://meta.fabricmc.net/v2/versions/installer")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let installer = installers
        .as_array()
        .and_then(|v| v.iter().find(|x| x["stable"] == true).or_else(|| v.first()))
        .and_then(|v| v["version"].as_str())
        .ok_or("没有可用的 Fabric 安装器")?;
    download(&format!("https://maven.fabricmc.net/net/fabricmc/fabric-installer/{installer}/fabric-installer-{installer}.jar"), temp).await?;
    run_installer(
        java,
        temp,
        &[
            "client".into(),
            "-dir".into(),
            root.to_string_lossy().into_owned(),
            "-mcversion".into(),
            game.into(),
            "-loader".into(),
            loader.into(),
            "-noprofile".into(),
        ],
        root,
    )
    .await?;
    Ok(LoaderInstallResult {
        loader: "Fabric".into(),
        loader_version: loader.into(),
        launch_version: format!("fabric-loader-{loader}-{game}"),
    })
}

async fn forge(
    game: &str,
    build: &str,
    root: &Path,
    java: &str,
    temp: &Path,
) -> Result<LoaderInstallResult, String> {
    let full = format!("{game}-{build}");
    download(&format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{full}/forge-{full}-installer.jar"), temp).await?;
    run_installer(
        java,
        temp,
        &[
            "--installClient".into(),
            root.to_string_lossy().into_owned(),
        ],
        root,
    )
    .await?;
    Ok(LoaderInstallResult {
        loader: "Forge".into(),
        loader_version: build.into(),
        launch_version: format!("{game}-forge-{build}"),
    })
}

async fn neoforge(
    _game: &str,
    version: &str,
    root: &Path,
    java: &str,
    temp: &Path,
) -> Result<LoaderInstallResult, String> {
    download(&format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"), temp).await?;
    run_installer(
        java,
        temp,
        &[
            "--installClient".into(),
            root.to_string_lossy().into_owned(),
        ],
        root,
    )
    .await?;
    Ok(LoaderInstallResult {
        loader: "NeoForge".into(),
        loader_version: version.into(),
        launch_version: format!("neoforge-{version}"),
    })
}

#[tauri::command]
pub async fn install_loader(
    instance_path: String,
    game_version: String,
    loader: String,
    loader_version: String,
    java_path: String,
) -> Result<LoaderInstallResult, String> {
    let _guard = LOADER_INSTALL.lock().await;
    let root = PathBuf::from(instance_path);
    if !root.is_absolute()
        || !root
            .join("versions")
            .join(&game_version)
            .join(".verdant-installed")
            .is_file()
    {
        return Err("实例未完整安装，无法添加模组加载器".into());
    }
    if installed_loader(&root).await? {
        return Err("当前实例已经安装模组加载器，不能重复安装或叠加其他加载器".into());
    }
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let temp = root.join(format!(".catcl-{}-installer.jar", loader.to_lowercase()));
    let available = list_loader_versions(game_version.clone(), loader.clone()).await?;
    if !available.iter().any(|version| version == &loader_version) {
        return Err("所选加载器版本不存在或与 Minecraft 版本不兼容".into());
    }
    let result = match loader.as_str() {
        "fabric" => fabric(&game_version, &loader_version, &root, &java_path, &temp).await,
        "forge" => forge(&game_version, &loader_version, &root, &java_path, &temp).await,
        "neoforge" => neoforge(&game_version, &loader_version, &root, &java_path, &temp).await,
        _ => Err("不支持的模组加载器".into()),
    };
    let _ = tokio::fs::remove_file(temp).await;
    if let Ok(installed) = &result {
        let metadata = serde_json::to_vec_pretty(installed).map_err(|e| e.to_string())?;
        tokio::fs::write(root.join(".catcl-loader.json"), metadata)
            .await
            .map_err(|e| format!("无法记录加载器状态：{e}"))?;
    }
    result
}

async fn installed_loader(root: &Path) -> Result<bool, String> {
    if root.join(".catcl-loader.json").is_file() {
        return Ok(true);
    }
    let mut entries = match tokio::fs::read_dir(root.join("versions")).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("fabric-loader-")
            || name.starts_with("neoforge-")
            || name.contains("-forge-")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_maven_versions() {
        assert_eq!(
            xml_versions("<versions><version>1.0</version><version>2.0</version></versions>"),
            ["1.0", "2.0"]
        );
    }
}
