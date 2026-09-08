use crate::launch::GameState;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tauri::Manager;
use tauri::State;

fn loader_from_id(id: &str) -> Option<&'static str> {
    let id = id.to_ascii_lowercase();
    if id.contains("neoforge") {
        Some("NeoForge")
    } else if id.contains("fabric-loader") || id.starts_with("fabric-") {
        Some("Fabric")
    } else if id.contains("forge") {
        Some("Forge")
    } else {
        None
    }
}

fn inspect_instance_directory(path: &std::path::Path) -> Option<GameInstance> {
    let roots = [path.join("versions"), path.join(".minecraft").join("versions")];
    let versions = roots.iter().find(|root| root.is_dir())?;
    let mut detected: Vec<(String, Option<String>, Option<String>, std::time::SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(versions).ok()?.flatten() {
        let version_dir = entry.path();
        if !version_dir.is_dir() {
            continue;
        }
        let fallback = entry.file_name().to_string_lossy().into_owned();
        let json_path = version_dir.join(format!("{fallback}.json"));
        let bytes = std::fs::read(&json_path).ok()?;
        let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let id = json["id"].as_str().unwrap_or(&fallback).to_owned();
        let inherited = json["inheritsFrom"].as_str().map(str::to_owned);
        let loader = loader_from_id(&id).map(str::to_owned);
        let modified = std::fs::metadata(&json_path)
            .and_then(|value| value.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        detected.push((id, inherited, loader, modified));
    }
    if detected.is_empty() {
        return None;
    }
    detected.sort_by_key(|item| std::cmp::Reverse(item.3));
    let selected = detected.iter().find(|item| item.2.is_some()).unwrap_or(&detected[0]);
    let version = selected.1.clone().unwrap_or_else(|| selected.0.clone());
    let name = path.file_name()?.to_string_lossy().trim().to_owned();
    if name.is_empty() {
        return None;
    }
    Some(GameInstance {
        name,
        version,
        path: path.to_string_lossy().into_owned(),
        loader: selected.2.clone(),
        launch_version: selected.2.as_ref().map(|_| selected.0.clone()),
    })
}

#[tauri::command]
pub async fn scan_instance_folder(path: String) -> Result<Vec<GameInstance>, String> {
    let root = PathBuf::from(path.trim());
    if !root.is_absolute() || !root.is_dir() {
        return Err("请选择有效的实例文件夹".into());
    }
    tokio::task::spawn_blocking(move || {
        let mut instances = Vec::new();
        if let Some(instance) = inspect_instance_directory(&root) {
            instances.push(instance);
        } else {
            let entries = std::fs::read_dir(&root).map_err(|error| error.to_string())?;
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    if let Some(instance) = inspect_instance_directory(&entry.path()) {
                        instances.push(instance);
                    }
                }
            }
        }
        instances.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        Ok(instances)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInstance {
    pub name: String,
    pub version: String,
    pub path: String,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub launch_version: Option<String>,
}

static SETTINGS_WRITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub java_path: String,
    #[serde(default = "default_offline_name")]
    pub offline_name: String,
    #[serde(default = "default_account_mode")]
    pub account_mode: String,
    #[serde(default)]
    pub microsoft_client_id: String,
    pub memory_mb: u32,
    pub game_dir: String,
    pub selected_version: Option<String>,
    pub show_snapshots: bool,
    #[serde(default)]
    pub instances: Vec<GameInstance>,
    #[serde(default)]
    pub selected_instance: Option<String>,
    #[serde(default)]
    pub experimental_hotset: bool,
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        validate_offline_name(&self.offline_name)?;
        if !matches!(self.account_mode.as_str(), "offline" | "littleskin" | "microsoft") {
            return Err("不支持的账户类型".into());
        }
        if !(1024..=32768).contains(&self.memory_mb) {
            return Err("内存必须在 1024–32768 MB 之间".into());
        }
        if self.java_path.trim().is_empty() {
            return Err("Java 路径不能为空".into());
        }
        if !PathBuf::from(&self.game_dir).is_absolute() {
            return Err("游戏目录必须是绝对路径".into());
        }
        let mut names = std::collections::HashSet::new();
        let mut paths: Vec<String> = Vec::new();
        for instance in &self.instances {
            let name = instance.name.trim();
            if name.is_empty() || name.len() > 40 {
                return Err("实例名称须为 1–40 个字符".into());
            }
            if !names.insert(name.to_lowercase()) {
                return Err(format!("实例名称不能重复：{name}"));
            }
            let path = PathBuf::from(instance.path.trim());
            if !path.is_absolute() {
                return Err(format!("实例目录必须是绝对路径：{}", instance.path));
            }
            let normalized = instance
                .path
                .trim()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase();
            if paths.iter().any(|existing| {
                normalized == *existing
                    || normalized.starts_with(&format!("{existing}\\"))
                    || existing.starts_with(&format!("{normalized}\\"))
            }) {
                return Err("每个实例必须使用互不嵌套的独立文件夹".into());
            }
            paths.push(normalized);
        }
        if self.selected_instance.as_ref().is_some_and(|selected| {
            !self
                .instances
                .iter()
                .any(|instance| &instance.name == selected)
        }) {
            return Err("选中的实例不存在".into());
        }
        Ok(())
    }
}

fn default_offline_name() -> String {
    "Player".into()
}

fn default_account_mode() -> String { "offline".into() }

pub(crate) fn validate_offline_name(name: &str) -> Result<(), String> {
    if !(3..=16).contains(&name.len())
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("离线玩家名须为 3–16 位英文字母、数字或下划线".into());
    }
    Ok(())
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("settings.json"))
}

#[tauri::command]
pub async fn load_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    let path = settings_path(&app)?;
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let settings: Settings = serde_json::from_slice(&bytes)
                .map_err(|e| format!("设置文件格式错误，原文件已保留：{e}"))?;
            settings.validate()?;
            Ok(settings)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings {
            java_path: "java".into(),
            offline_name: default_offline_name(),
            account_mode: default_account_mode(),
            microsoft_client_id: String::new(),
            memory_mb: 4096,
            game_dir: app
                .path()
                .document_dir()
                .map_err(|e| e.to_string())?
                .join("CatCL")
                .to_string_lossy()
                .into_owned(),
            selected_version: None,
            show_snapshots: false,
            experimental_hotset: false,
            instances: Vec::new(),
            selected_instance: None,
        }),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    settings.validate()?;
    let _guard = SETTINGS_WRITE.lock().await;
    let path = settings_path(&app)?;
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    tokio::fs::write(&temp, bytes)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(temp, path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_instance(
    app: tauri::AppHandle,
    game: State<'_, GameState>,
    name: String,
) -> Result<Settings, String> {
    if game.is_running() {
        return Err("游戏运行时不能删除实例".into());
    }
    let mut settings = load_settings(app.clone()).await?;
    let index = settings
        .instances
        .iter()
        .position(|instance| instance.name == name)
        .ok_or("实例不存在或已被删除")?;
    let instance = &settings.instances[index];
    let root = PathBuf::from(&instance.path);
    if !root.is_absolute() || root.components().count() < 3 {
        return Err("拒绝删除不安全的实例路径".into());
    }
    let metadata = tokio::fs::symlink_metadata(&root)
        .await
        .map_err(|e| format!("无法读取实例目录：{e}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("拒绝删除符号链接或非文件夹实例".into());
    }
    let marker = root
        .join("versions")
        .join(&instance.version)
        .join(".verdant-installed");
    if !marker.is_file() {
        return Err("目录缺少 CatCL 实例标记，已取消删除以保护文件".into());
    }
    let canonical_root = tokio::fs::canonicalize(&root)
        .await
        .map_err(|e| e.to_string())?;
    let parent = root.parent().ok_or("实例目录不能是磁盘根目录")?;
    let canonical_parent = tokio::fs::canonicalize(parent)
        .await
        .map_err(|e| e.to_string())?;
    if canonical_root.parent() != Some(canonical_parent.as_path()) {
        return Err("实例路径边界校验失败，已取消删除".into());
    }
    let mut suffix = 1u32;
    let staged = loop {
        let candidate =
            canonical_parent.join(format!(".catcl-deleting-{}-{suffix}", std::process::id()));
        if tokio::fs::symlink_metadata(&candidate).await.is_err() {
            break candidate;
        }
        suffix = suffix.checked_add(1).ok_or("无法生成安全的删除暂存路径")?;
    };
    tokio::fs::rename(&canonical_root, &staged)
        .await
        .map_err(|e| format!("无法暂存待删除实例：{e}"))?;
    settings.instances.remove(index);
    if settings.selected_instance.as_deref() == Some(name.as_str()) {
        settings.selected_instance = None;
    }
    if let Err(error) = save_settings(app, settings.clone()).await {
        let _ = tokio::fs::rename(&staged, &canonical_root).await;
        return Err(error);
    }
    tokio::fs::remove_dir_all(&staged).await.map_err(|e| {
        format!(
            "实例已从列表移除，但暂存文件清理失败（{}）：{e}",
            staged.display()
        )
    })?;
    Ok(settings)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaInfo {
    pub path: String,
    pub version: String,
    pub major: u32,
    pub is64_bit: bool,
}

pub(crate) fn parse_java_major(output: &str) -> Option<u32> {
    let version = output.split("version \"").nth(1)?.split('"').next()?;
    let mut parts = version.split(['.', '-', '+']);
    match parts.next()?.parse().ok()? {
        1 => parts.next()?.parse().ok(),
        n => Some(n),
    }
}

#[tauri::command]
pub async fn detect_java(path: String) -> Result<JavaInfo, String> {
    inspect_java(path).await
}

pub(crate) async fn inspect_java(path: String) -> Result<JavaInfo, String> {
    if path.trim().is_empty() {
        return Err("请填写 Java 路径".into());
    }
    let mut command = tokio::process::Command::new(path.trim());
    command.arg("-version").kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| "Java 检测超时".to_string())?
        .map_err(|e| format!("无法执行 Java，请检查路径：{e}"))?;
    if !output.status.success() {
        return Err("Java 执行失败，请选择有效的 java.exe".into());
    }
    let version = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let major = parse_java_major(&version).ok_or("无法识别 Java 版本")?;
    let is64_bit =
        version.contains("64-Bit") || version.contains("64-bit") || version.contains("aarch64");
    Ok(JavaInfo {
        path,
        version: version.trim().into(),
        major,
        is64_bit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_loader_version_inside_an_instance_folder() {
        let root = std::env::temp_dir().join(format!("catcl-instance-scan-{}", std::process::id()));
        let instance = root.join("large-pack");
        let vanilla = instance.join("versions/1.20.1");
        let fabric = instance.join("versions/fabric-loader-0.16.14-1.20.1");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&vanilla).unwrap();
        std::fs::create_dir_all(&fabric).unwrap();
        std::fs::write(vanilla.join("1.20.1.json"), br#"{"id":"1.20.1","type":"release"}"#).unwrap();
        std::fs::write(
            fabric.join("fabric-loader-0.16.14-1.20.1.json"),
            br#"{"id":"fabric-loader-0.16.14-1.20.1","inheritsFrom":"1.20.1"}"#,
        ).unwrap();
        let found = inspect_instance_directory(&instance).unwrap();
        assert_eq!(found.name, "large-pack");
        assert_eq!(found.version, "1.20.1");
        assert_eq!(found.loader.as_deref(), Some("Fabric"));
        assert_eq!(found.launch_version.as_deref(), Some("fabric-loader-0.16.14-1.20.1"));
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn parses_java_versions() {
        assert_eq!(parse_java_major("java version \"1.8.0_421\""), Some(8));
        assert_eq!(
            parse_java_major("openjdk version \"21.0.4\" 2024-07-16"),
            Some(21)
        );
        assert_eq!(parse_java_major("openjdk version \"25-ea\""), Some(25));
        assert_eq!(parse_java_major("unexpected output"), None);
    }
    #[test]
    fn rejects_invalid_settings() {
        let mut settings = Settings {
            java_path: "java".into(),
            offline_name: "Player".into(),
            account_mode: "offline".into(),
            microsoft_client_id: String::new(),
            memory_mb: 4096,
            game_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            selected_version: None,
            show_snapshots: false,
            experimental_hotset: false,
            instances: Vec::new(),
            selected_instance: None,
        };
        assert!(settings.validate().is_ok());
        settings.memory_mb = 0;
        assert!(settings.validate().is_err());
        settings.memory_mb = 4096;
        settings.offline_name = "a!".into();
        assert!(settings.validate().is_err());
        settings.offline_name = "Valid_Name".into();
        settings.game_dir = "relative/path".into();
        assert!(settings.validate().is_err());
    }
    #[test]
    fn rejects_duplicate_instance_names_and_paths() {
        let root = std::env::temp_dir();
        let mut settings = Settings {
            java_path: "java".into(),
            offline_name: "Player".into(),
            account_mode: "offline".into(),
            microsoft_client_id: String::new(),
            memory_mb: 4096,
            game_dir: root.to_string_lossy().into_owned(),
            selected_version: Some("1.21.1".into()),
            show_snapshots: false,
            experimental_hotset: false,
            instances: vec![
                GameInstance {
                    name: "生存".into(),
                    version: "1.21.1".into(),
                    path: root.join("one").to_string_lossy().into_owned(),
                    loader: None,
                    launch_version: None,
                },
                GameInstance {
                    name: "生存".into(),
                    version: "1.21.1".into(),
                    path: root.join("two").to_string_lossy().into_owned(),
                    loader: None,
                    launch_version: None,
                },
            ],
            selected_instance: Some("生存".into()),
        };
        assert!(settings.validate().unwrap_err().contains("名称不能重复"));
        settings.instances[1].name = "创造".into();
        settings.instances[1].path = settings.instances[0].path.clone();
        assert!(settings.validate().unwrap_err().contains("独立文件夹"));
    }
}
