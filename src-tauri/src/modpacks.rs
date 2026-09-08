use serde::Serialize;
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::Emitter;

const MAX_ENTRIES: usize = 100_000;
const MAX_METADATA: u64 = 2 * 1024 * 1024;
const MAX_APPLIED_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_NESTED_PACK: u64 = 8 * 1024 * 1024 * 1024;
static TEMP_PACK_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackInfo {
    format: String,
    name: String,
    version: String,
    minecraft_version: String,
    loaders: Vec<String>,
    file_count: usize,
    warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    applied_files: usize,
    downloaded_files: usize,
    skipped_files: usize,
    warnings: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModpackProgress {
    completed: usize,
    total: usize,
    message: String,
}

fn emit_progress(
    app: &tauri::AppHandle,
    completed: usize,
    total: usize,
    message: impl Into<String>,
) {
    let _ = app.emit(
        "modpack-progress",
        ModpackProgress {
            completed,
            total,
            message: message.into(),
        },
    );
}

#[derive(Clone)]
struct RemoteFile {
    path: String,
    url: String,
    sha1: Option<String>,
}

fn read_json(zip: &mut zip::ZipArchive<File>, name: &str) -> Result<Option<Value>, String> {
    let Ok(mut entry) = zip.by_name(name) else {
        return Ok(None);
    };
    if entry.size() > MAX_METADATA {
        return Err(format!("整合包元数据过大：{name}"));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| format!("无法解析 {name}：{e}"))
}

fn read_json_suffix(
    zip: &mut zip::ZipArchive<File>,
    suffix: &str,
) -> Result<Option<Value>, String> {
    for index in 0..zip.len() {
        let matches = {
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            entry.name().replace('\\', "/").ends_with(suffix)
        };
        if matches {
            let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
            if entry.size() > MAX_METADATA {
                return Err(format!("整合包元数据过大：{suffix}"));
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("无法解析 {suffix}：{e}"));
        }
    }
    Ok(None)
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn first_text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn plausible_minecraft_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn loader_from_library(name: &str, loaders: &mut Vec<String>) {
    let parts: Vec<&str> = name.split(':').collect();
    let loader = match parts.as_slice() {
        ["net.fabricmc", "fabric-loader", version, ..] => Some(format!("fabric-loader {version}")),
        ["net.neoforged", "neoforge", version, ..] => Some(format!("neoforge {version}")),
        ["net.minecraftforge", "forge", version, ..] => Some(format!(
            "forge {}",
            version.rsplit('-').next().unwrap_or(version)
        )),
        _ => None,
    };
    if let Some(loader) = loader {
        if !loaders.contains(&loader) {
            loaders.push(loader);
        }
    }
}

fn infer_pcl_version(
    zip: &mut zip::ZipArchive<File>,
) -> Result<(String, Vec<String>, Option<String>), String> {
    let mut minecraft_version = String::new();
    let mut loaders = Vec::new();
    let mut instance_name = None;
    for index in 0..zip.len() {
        let candidate = {
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            let normalized = entry.name().replace('\\', "/");
            let lower = normalized.to_ascii_lowercase();
            (lower.contains("/versions/") || lower.starts_with("versions/"))
                && lower.ends_with(".json")
                && entry.size() <= MAX_METADATA
        };
        if !candidate {
            continue;
        }
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let normalized = entry.name().replace('\\', "/");
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let Ok(version): Result<Value, _> = serde_json::from_slice(&bytes) else {
            continue;
        };
        let id = text(version.get("id"));
        if instance_name.is_none() {
            instance_name = normalized
                .split("/versions/")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .filter(|name| !name.is_empty())
                .map(str::to_owned);
        }
        let inherited = first_text(&version, &["inheritsFrom", "clientVersion", "jar"]);
        if plausible_minecraft_version(&inherited) {
            minecraft_version = inherited;
        } else if minecraft_version.is_empty() && plausible_minecraft_version(&id) {
            minecraft_version = id;
        }
        for library in version["libraries"].as_array().into_iter().flatten() {
            if let Some(name) = library["name"].as_str() {
                loader_from_library(name, &mut loaders);
            }
        }
    }
    Ok((minecraft_version, loaders, instance_name))
}

struct TemporaryPack(std::path::PathBuf);
impl Drop for TemporaryPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn extract_nested_pcl_pack(path: &Path) -> Result<Option<TemporaryPack>, String> {
    let file = File::open(path).map_err(|e| format!("无法打开整合包：{e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("不是有效的 ZIP 压缩包：{e}"))?;
    let has_pcl_setup = (0..zip.len()).any(|index| {
        zip.by_index(index).is_ok_and(|entry| {
            entry
                .name()
                .replace('\\', "/")
                .to_ascii_lowercase()
                .ends_with("pcl/setup.ini")
        })
    });
    if !has_pcl_setup {
        return Ok(None);
    }
    let candidate = (0..zip.len()).find(|index| {
        zip.by_index(*index).is_ok_and(|entry| {
            let name = entry.name().replace('\\', "/");
            let lower = name.to_ascii_lowercase();
            !entry.is_dir()
                && !name.contains('/')
                && (lower.ends_with(".mrpack") || lower.ends_with(".zip"))
        })
    });
    let Some(index) = candidate else {
        return Ok(None);
    };
    let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
    if entry.size() == 0 || entry.size() > MAX_NESTED_PACK {
        return Err("PCL 封装内的整合包为空或超过 8 GB 安全上限".into());
    }
    let extension = if entry.name().to_ascii_lowercase().ends_with(".mrpack") {
        "mrpack"
    } else {
        "zip"
    };
    let id = TEMP_PACK_ID.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "catcl-nested-pack-{}-{id}.{extension}",
        std::process::id()
    ));
    let mut output = File::create(&target).map_err(|e| format!("无法暂存内层整合包：{e}"))?;
    std::io::copy(&mut entry, &mut output).map_err(|e| format!("无法读取内层整合包：{e}"))?;
    output.flush().map_err(|e| e.to_string())?;
    Ok(Some(TemporaryPack(target)))
}

fn inspect(path: &Path) -> Result<ModpackInfo, String> {
    inspect_with_depth(path, 0)
}

fn inspect_with_depth(path: &Path, depth: usize) -> Result<ModpackInfo, String> {
    if depth > 2 {
        return Err("整合包嵌套层数超过安全限制".into());
    }
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("zip") && !ext.eq_ignore_ascii_case("mrpack"))
    {
        return Err("请选择 .zip 或 .mrpack 整合包".into());
    }
    if let Some(nested) = extract_nested_pcl_pack(path)? {
        let mut info = inspect_with_depth(&nested.0, depth + 1)?;
        info.format = format!("PCL 封装 / {}", info.format);
        info.warnings
            .push("已自动读取 PCL 外层 ZIP 中的内嵌整合包。".into());
        return Ok(info);
    }
    let file = File::open(path).map_err(|e| format!("无法打开整合包：{e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("不是有效的 ZIP 压缩包：{e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("压缩包文件数量过多：{}", zip.len()));
    }
    let file_count = zip.len();
    let mut warnings = Vec::new();

    if let Some(index) = read_json(&mut zip, "modrinth.index.json")? {
        let dependencies = index["dependencies"].as_object();
        let minecraft_version = dependencies
            .and_then(|items| items.get("minecraft"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let loaders = dependencies
            .into_iter()
            .flat_map(|items| items.iter())
            .filter(|(key, _)| key.as_str() != "minecraft")
            .map(|(key, value)| format!("{key} {}", value.as_str().unwrap_or("未知版本")))
            .collect();
        return Ok(ModpackInfo {
            format: "Modrinth".into(),
            name: text(index.get("name")),
            version: text(index.get("versionId")),
            minecraft_version,
            loaders,
            file_count,
            warnings,
        });
    }

    if let Some(manifest) = read_json(&mut zip, "manifest.json")? {
        if manifest.get("minecraft").is_some() && manifest.get("files").is_some() {
            let minecraft = &manifest["minecraft"];
            let loaders = minecraft["modLoaders"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|loader| loader["id"].as_str().map(str::to_owned))
                .collect();
            return Ok(ModpackInfo {
                format: "CurseForge".into(),
                name: text(manifest.get("name")),
                version: text(manifest.get("version")),
                minecraft_version: text(minecraft.get("version")),
                loaders,
                file_count,
                warnings,
            });
        }
    }

    if let Some(pack) = read_json(&mut zip, "mmc-pack.json")? {
        let mut minecraft_version = String::new();
        let mut loaders = Vec::new();
        for component in pack["components"].as_array().into_iter().flatten() {
            let uid = component["uid"].as_str().unwrap_or_default();
            let version = component["version"].as_str().unwrap_or_default();
            if uid == "net.minecraft" {
                minecraft_version = version.into();
            } else if uid.contains("forge") || uid.contains("fabric") || uid.contains("quilt") {
                loaders.push(format!(
                    "{} {version}",
                    uid.rsplit('.').next().unwrap_or(uid)
                ));
            }
        }
        warnings.push("Prism/MultiMC 的实例名称通常保存在 instance.cfg，导入时可重新命名。".into());
        return Ok(ModpackInfo {
            format: "Prism / MultiMC".into(),
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            version: String::new(),
            minecraft_version,
            loaders,
            file_count,
            warnings,
        });
    }

    if let Some(meta) = read_json_suffix(&mut zip, "mcbbs.packmeta")? {
        let mut minecraft_version = first_text(&meta, &["mcVersion", "minecraftVersion"]);
        if minecraft_version.is_empty() {
            minecraft_version = first_text(&meta["minecraft"], &["version", "mcVersion"]);
        }
        let mut loaders = Vec::new();
        for addon in meta["addons"].as_array().into_iter().flatten() {
            let id = addon["id"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let version = addon["version"].as_str().unwrap_or_default();
            if id == "game" || id == "minecraft" {
                minecraft_version = version.into();
            } else if ["forge", "fabric", "fabric-loader", "neoforge", "quilt"]
                .contains(&id.as_str())
            {
                loaders.push(format!("{id} {version}"));
            }
        }
        return Ok(ModpackInfo {
            format: "PCL / MCBBS".into(),
            name: first_text(&meta, &["name", "title", "packName"]),
            version: first_text(&meta, &["version", "versionId", "packVersion"]),
            minecraft_version,
            loaders,
            file_count,
            warnings,
        });
    }

    let mut has_mods = false;
    let mut has_config = false;
    let mut has_pcl = false;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/").to_ascii_lowercase();
        has_mods |= name.starts_with("mods/") || name.contains("/mods/");
        has_config |= name.starts_with("config/") || name.contains("/config/");
        has_pcl |= name.ends_with("pcl/setup.ini") || name.contains("/.minecraft/");
    }
    if has_pcl {
        let (minecraft_version, loaders, instance_name) = infer_pcl_version(&mut zip)?;
        if minecraft_version.is_empty() {
            warnings.push(
                "已识别 PCL 实例结构，但版本 JSON 未声明可识别的 Minecraft 基础版本；应用前需手动确认目标版本。".into(),
            );
        } else {
            warnings.push(format!(
                "已从 PCL 实例版本 JSON 识别 Minecraft {minecraft_version}。"
            ));
        }
        return Ok(ModpackInfo {
            format: "PCL 实例 ZIP".into(),
            name: instance_name.unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }),
            version: String::new(),
            minecraft_version,
            loaders,
            file_count,
            warnings,
        });
    }
    if has_mods || has_config {
        warnings.push("缺少标准整合包清单，无法可靠确定 Minecraft 与加载器版本。".into());
        return Ok(ModpackInfo {
            format: "通用 ZIP".into(),
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            version: String::new(),
            minecraft_version: String::new(),
            loaders: Vec::new(),
            file_count,
            warnings,
        });
    }
    Err("未识别到受支持的整合包清单或 mods/config 目录".into())
}

#[tauri::command]
pub async fn inspect_modpack(path: String) -> Result<ModpackInfo, String> {
    let path = std::path::PathBuf::from(path);
    if !path.is_absolute() {
        return Err("整合包路径必须是绝对路径".into());
    }
    tokio::task::spawn_blocking(move || inspect(&path))
        .await
        .map_err(|e| e.to_string())?
}

fn extract_prefix(
    archive: &Path,
    game: &Path,
    prefixes: &[&str],
    generic: bool,
) -> Result<usize, String> {
    let mut zip = zip::ZipArchive::new(File::open(archive).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err("压缩包文件数量过多".into());
    }
    let mut count = 0;
    let mut total = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        total = total
            .checked_add(entry.size())
            .ok_or("整合包展开大小溢出")?;
        if total > MAX_APPLIED_BYTES {
            return Err("整合包展开后超过 20 GB 安全上限".into());
        }
        let enclosed = entry.enclosed_name().ok_or("压缩包包含不安全路径")?;
        let normalized = enclosed.to_string_lossy().replace('\\', "/");
        let relative = prefixes
            .iter()
            .find_map(|prefix| {
                normalized
                    .find(prefix)
                    .map(|index| &normalized[index + prefix.len()..])
            })
            .or_else(|| {
                (generic
                    && [
                        "mods/",
                        "config/",
                        "resourcepacks/",
                        "shaderpacks/",
                        "defaultconfigs/",
                    ]
                    .iter()
                    .any(|prefix| normalized.starts_with(prefix)))
                .then_some(normalized.as_str())
            });
        let Some(relative) = relative else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let target = game.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(target.parent().ok_or("无效的整合包文件路径")?)
            .map_err(|e| e.to_string())?;
        let mut output = File::create(target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

#[tauri::command]
pub async fn apply_modpack(
    app: tauri::AppHandle,
    path: String,
    instance_path: String,
    game_version: String,
) -> Result<ApplyResult, String> {
    let outer_archive = std::path::PathBuf::from(path);
    let root = std::path::PathBuf::from(instance_path);
    if !outer_archive.is_absolute() || !root.is_absolute() {
        return Err("整合包和实例目录必须是绝对路径".into());
    }
    if !root
        .join("versions")
        .join(&game_version)
        .join(".verdant-installed")
        .is_file()
    {
        return Err("目标实例尚未完整安装".into());
    }
    let nested_source = outer_archive.clone();
    let nested = tokio::task::spawn_blocking(move || extract_nested_pcl_pack(&nested_source))
        .await
        .map_err(|e| e.to_string())??;
    let archive = nested
        .as_ref()
        .map(|temporary| temporary.0.clone())
        .unwrap_or(outer_archive);
    let info_path = archive.clone();
    let info = tokio::task::spawn_blocking(move || inspect(&info_path))
        .await
        .map_err(|e| e.to_string())??;
    if !info.minecraft_version.is_empty() && info.minecraft_version != game_version {
        return Err(format!(
            "整合包需要 Minecraft {}，当前实例是 {game_version}",
            info.minecraft_version
        ));
    }
    let game = root.join("game");
    tokio::fs::create_dir_all(&game)
        .await
        .map_err(|e| e.to_string())?;
    let mut remote = Vec::new();
    let mut skipped = 0;
    let mut warnings = info.warnings;
    if info.format == "Modrinth" {
        let mut zip = zip::ZipArchive::new(File::open(&archive).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let index = read_json(&mut zip, "modrinth.index.json")?.ok_or("缺少 Modrinth 清单")?;
        for file in index["files"].as_array().into_iter().flatten() {
            let Some(path) = file["path"].as_str() else {
                skipped += 1;
                continue;
            };
            let Some(url) = file["downloads"]
                .as_array()
                .and_then(|v| v.first())
                .and_then(Value::as_str)
            else {
                skipped += 1;
                continue;
            };
            if !url.starts_with("https://")
                || Path::new(path)
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("整合包包含不安全的下载路径或地址".into());
            }
            remote.push(RemoteFile {
                path: path.into(),
                url: url.into(),
                sha1: file["hashes"]["sha1"].as_str().map(str::to_owned),
            });
        }
    } else if info.format == "CurseForge" {
        let mut zip = zip::ZipArchive::new(File::open(&archive).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if let Some(manifest) = read_json(&mut zip, "manifest.json")? {
            skipped = manifest["files"].as_array().map_or(0, Vec::len);
        }
        if skipped > 0 {
            warnings.push(format!(
                "{skipped} 个 CurseForge 外部项目需要平台授权下载，本次只应用包内 overrides。"
            ));
        }
    }
    let extract_archive = archive.clone();
    let extract_game = game.clone();
    let format = info.format.clone();
    let total = remote.len() + 1;
    emit_progress(&app, 0, total, "正在解压整合包覆盖文件…");
    let applied = tokio::task::spawn_blocking(move || match format.as_str() {
        "Modrinth" => extract_prefix(
            &extract_archive,
            &extract_game,
            &["overrides/", "client-overrides/"],
            false,
        ),
        "CurseForge" => extract_prefix(&extract_archive, &extract_game, &["overrides/"], false),
        "Prism / MultiMC" => {
            extract_prefix(&extract_archive, &extract_game, &[".minecraft/"], false)
        }
        "PCL / MCBBS" | "PCL 实例 ZIP" => extract_prefix(
            &extract_archive,
            &extract_game,
            &["overrides/", ".minecraft/"],
            false,
        ),
        _ => extract_prefix(&extract_archive, &extract_game, &[], true),
    })
    .await
    .map_err(|e| e.to_string())??;
    emit_progress(&app, 1, total, format!("已应用 {applied} 个覆盖文件"));
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let mut downloaded = 0;
    for (index, file) in remote.into_iter().enumerate() {
        emit_progress(
            &app,
            index + 1,
            total,
            format!("正在下载整合包文件：{}", file.path),
        );
        let bytes = http
            .get(&file.url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > 2 * 1024 * 1024 * 1024 {
            return Err(format!("模组文件过大：{}", file.path));
        }
        if let Some(expected) = file.sha1 {
            use sha1::Digest;
            let actual = format!("{:x}", sha1::Sha1::digest(&bytes));
            if !actual.eq_ignore_ascii_case(&expected) {
                return Err(format!("模组校验失败：{}", file.path));
            }
        }
        let target = game.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        tokio::fs::create_dir_all(target.parent().ok_or("无效下载路径")?)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(target, bytes)
            .await
            .map_err(|e| e.to_string())?;
        downloaded += 1;
        emit_progress(
            &app,
            index + 2,
            total,
            format!("已下载 {downloaded} 个整合包文件"),
        );
    }
    emit_progress(&app, total, total, "整合包应用完成");
    Ok(ApplyResult {
        applied_files: applied,
        downloaded_files: downloaded,
        skipped_files: skipped,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn recognizes_modrinth_metadata() {
        let path = std::env::temp_dir().join(format!("catcl-pack-{}.zip", std::process::id()));
        let file = File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "modrinth.index.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(br#"{"name":"Test Pack","versionId":"1.0","dependencies":{"minecraft":"1.21.1","fabric-loader":"0.16.0"}}"#).unwrap();
        archive.finish().unwrap();
        let info = inspect(&path).unwrap();
        assert_eq!(info.format, "Modrinth");
        assert_eq!(info.minecraft_version, "1.21.1");
        assert_eq!(info.loaders, ["fabric-loader 0.16.0"]);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn recognizes_nested_pcl_mcbbs_metadata() {
        let path = std::env::temp_dir().join(format!("catcl-pcl-pack-{}.zip", std::process::id()));
        let file = File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "Pack/mcbbs.packmeta",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(br#"{"name":"PCL Pack","version":"2.0","addons":[{"id":"game","version":"1.20.1"},{"id":"forge","version":"47.3.0"}]}"#).unwrap();
        archive.finish().unwrap();
        let info = inspect(&path).unwrap();
        assert_eq!(info.format, "PCL / MCBBS");
        assert_eq!(info.minecraft_version, "1.20.1");
        assert_eq!(info.loaders, ["forge 47.3.0"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn infers_pcl_instance_version_and_loader_from_version_json() {
        let path =
            std::env::temp_dir().join(format!("catcl-pcl-instance-{}.zip", std::process::id()));
        let file = File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "MyPack/.minecraft/versions/暮色冒险/暮色冒险.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(br#"{"id":"fabric-loader-0.16.0-1.20.1","inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.0"}]}"#).unwrap();
        archive.finish().unwrap();
        let info = inspect(&path).unwrap();
        assert_eq!(info.format, "PCL 实例 ZIP");
        assert_eq!(info.name, "暮色冒险");
        assert_eq!(info.minecraft_version, "1.20.1");
        assert_eq!(info.loaders, ["fabric-loader 0.16.0"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn does_not_treat_an_unrelated_modpack_json_as_pcl_metadata() {
        let path =
            std::env::temp_dir().join(format!("catcl-generic-pack-{}.zip", std::process::id()));
        let file = File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "config/example/modpack.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(br#"{}"#).unwrap();
        archive.finish().unwrap();
        let info = inspect(&path).unwrap();
        assert_eq!(info.format, "通用 ZIP");
        assert!(!info.name.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_a_modrinth_pack_nested_in_a_pcl_wrapper() {
        let id = std::process::id();
        let inner_path = std::env::temp_dir().join(format!("catcl-inner-{id}.mrpack"));
        let inner = File::create(&inner_path).unwrap();
        let mut inner_archive = zip::ZipWriter::new(inner);
        inner_archive
            .start_file(
                "modrinth.index.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        inner_archive.write_all(br#"{"name":"Nested Pack","versionId":"3.5.2","dependencies":{"minecraft":"1.20.1","forge":"47.3.0"}}"#).unwrap();
        inner_archive.finish().unwrap();
        let inner_bytes = std::fs::read(&inner_path).unwrap();

        let outer_path = std::env::temp_dir().join(format!("catcl-outer-{id}.zip"));
        let outer = File::create(&outer_path).unwrap();
        let mut outer_archive = zip::ZipWriter::new(outer);
        outer_archive
            .start_file("modpack.mrpack", zip::write::SimpleFileOptions::default())
            .unwrap();
        outer_archive.write_all(&inner_bytes).unwrap();
        outer_archive
            .start_file("PCL/Setup.ini", zip::write::SimpleFileOptions::default())
            .unwrap();
        outer_archive.write_all(b"VersionArgumentIndie=1").unwrap();
        outer_archive.finish().unwrap();

        let info = inspect(&outer_path).unwrap();
        assert_eq!(info.format, "PCL 封装 / Modrinth");
        assert_eq!(info.name, "Nested Pack");
        assert_eq!(info.version, "3.5.2");
        assert_eq!(info.minecraft_version, "1.20.1");
        assert_eq!(info.loaders, ["forge 47.3.0"]);
        std::fs::remove_file(inner_path).unwrap();
        std::fs::remove_file(outer_path).unwrap();
    }
}
