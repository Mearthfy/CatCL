use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const LABEL_FILE: &str = ".verdant-world.json";
static WORLD_WRITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldInfo {
    id: String,
    name: String,
    folder: String,
    location: String,
    modified_at: u64,
}

#[derive(Serialize, Default)]
pub struct WorldLibrary {
    worlds: Vec<WorldInfo>,
    warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct WorldLabel {
    name: String,
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("名称需为 1–80 个字符，且不能包含换行或控制字符".into());
    }
    Ok(name.into())
}

fn game_root(root: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(root);
    if !path.is_absolute() {
        return Err("请先在设置中保存有效的游戏目录".into());
    }
    Ok(path)
}

fn resolve_world(root: &Path, id: &str) -> Result<PathBuf, String> {
    let relative = Path::new(id);
    if id.is_empty()
        || id.contains(':')
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("无效的世界标识".into());
    }
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let path = fs::canonicalize(root.join(relative)).map_err(|e| e.to_string())?;
    if !path.starts_with(&root) || !path.join("level.dat").is_file() {
        return Err("世界不在当前游戏目录中，或缺少 level.dat".into());
    }
    Ok(path)
}

fn read_world(root: &Path, path: &Path, location: &str) -> Result<WorldInfo, String> {
    let id = path
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let resolved = resolve_world(root, &id)?;
    let folder = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let name = fs::read(resolved.join(LABEL_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<WorldLabel>(&bytes).ok())
        .and_then(|label| validate_name(&label.name).ok())
        .unwrap_or_else(|| folder.clone());
    let modified_at = fs::metadata(resolved.join("level.dat"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(WorldInfo {
        id,
        name,
        folder,
        location: location.into(),
        modified_at,
    })
}

fn scan_saves(root: &Path, saves: &Path, location: &str, result: &mut WorldLibrary) {
    let entries = match fs::read_dir(saves) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            result
                .warnings
                .push(format!("无法读取 {}：{e}", saves.display()));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                result.warnings.push(e.to_string());
                continue;
            }
        };
        if entry.file_name().to_string_lossy().starts_with('.')
            || !entry.path().join("level.dat").is_file()
        {
            continue;
        }
        match read_world(root, &entry.path(), location) {
            Ok(world) => result.worlds.push(world),
            Err(e) => result.warnings.push(e),
        }
    }
}

fn scan(root: &Path) -> WorldLibrary {
    let mut result = WorldLibrary::default();
    scan_saves(root, &root.join("saves"), "公共存档", &mut result);
    for group in ["instances", "versions"] {
        if let Ok(entries) = fs::read_dir(root.join(group)) {
            for entry in entries.flatten() {
                scan_saves(
                    root,
                    &entry.path().join("saves"),
                    &entry.file_name().to_string_lossy(),
                    &mut result,
                );
            }
        }
    }
    result
        .worlds
        .sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then(a.name.cmp(&b.name)));
    result
}

#[tauri::command]
pub async fn list_worlds(game_dir: String) -> Result<WorldLibrary, String> {
    let root = game_root(&game_dir)?;
    tokio::task::spawn_blocking(move || scan(&root))
        .await
        .map_err(|e| e.to_string())
}

fn write_label(world: &Path, name: &str) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&WorldLabel {
        name: validate_name(name)?,
    })
    .map_err(|e| e.to_string())?;
    let target = world.join(LABEL_FILE);
    let temp = world.join(".verdant-world.tmp");
    // Do not follow a foreign sidecar symlink when editing existing worlds.
    for path in [&target, &temp] {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("世界名称文件不能是符号链接".into());
        }
    }
    fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&temp, &target).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_world(game_dir: String, id: String, name: String) -> Result<(), String> {
    let _guard = WORLD_WRITE.lock().await;
    let root = game_root(&game_dir)?;
    let name = validate_name(&name)?;
    tokio::task::spawn_blocking(move || write_label(&resolve_world(&root, &id)?, &name))
        .await
        .map_err(|e| e.to_string())?
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err("导入目录不能包含符号链接，请先复制为普通文件".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if fs::symlink_metadata(entry.path())
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("导入目录不能包含目录联接或其他重解析点".into());
            }
        }
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn import_folder(root: &Path, source: &Path, name: &str) -> Result<(), String> {
    let name = validate_name(name)?;
    let source = fs::canonicalize(source).map_err(|e| format!("无法读取世界文件夹：{e}"))?;
    if !source.join("level.dat").is_file() {
        return Err("请选择直接包含 level.dat 的已解压世界文件夹".into());
    }
    // Reject recursive imports before creating anything inside the source world.
    for ancestor in root.ancestors() {
        if ancestor.exists() {
            if fs::canonicalize(ancestor)
                .map_err(|e| e.to_string())?
                .starts_with(&source)
            {
                return Err("导入目标不能位于源世界文件夹中".into());
            }
            break;
        }
    }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let saves = root.join("saves");
    fs::create_dir_all(&saves).map_err(|e| e.to_string())?;
    let saves = fs::canonicalize(saves).map_err(|e| e.to_string())?;
    if !saves.starts_with(&root) || saves.starts_with(&source) {
        return Err("导入目标不能位于源世界文件夹中或链接到游戏目录外".into());
    }
    let key = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let stage = saves.join(format!(".import-{key}"));
    let target = saves.join(format!("world-{key}"));
    let result = (|| {
        copy_tree(&source, &stage)?;
        write_label(&stage, &name)?;
        fs::rename(&stage, target).map_err(|e| e.to_string())
    })();
    if result.is_err() && stage.starts_with(&saves) && stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[tauri::command]
pub async fn import_world(game_dir: String, source: String, name: String) -> Result<(), String> {
    let _guard = WORLD_WRITE.lock().await;
    let root = game_root(&game_dir)?;
    let source = PathBuf::from(source);
    if !source.is_absolute() {
        return Err("请填写世界文件夹的完整路径".into());
    }
    tokio::task::spawn_blocking(move || import_folder(&root, &source, &name))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn open_world_folder(game_dir: String, id: String) -> Result<(), String> {
    let root = game_root(&game_dir)?;
    let path = tokio::task::spawn_blocking(move || {
        resolve_world(&root, &id)?;
        Ok::<_, String>(root.join(id))
    })
    .await
    .map_err(|e| e.to_string())??;
    #[cfg(windows)]
    {
        tokio::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("目前仅支持 Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let key = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("verdant-worlds-{}-{key}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn world(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.join("region")).unwrap();
            fs::write(path.join("level.dat"), b"original world metadata").unwrap();
            fs::write(path.join("region/r.0.0.mca"), b"original chunks").unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let temp = std::env::temp_dir();
            if self.0.starts_with(temp)
                && self
                    .0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("verdant-worlds-")
            {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }
    #[test]
    fn scans_all_save_locations_and_ignores_incomplete_worlds() {
        let fixture = Fixture::new();
        fixture.world("saves/town");
        fixture.world("instances/1.21.1/saves/island");
        fixture.world("versions/1.20.1/saves/castle");
        fixture.world("saves/.import-pending");
        fs::create_dir_all(fixture.0.join("saves/empty")).unwrap();
        let library = scan(&fixture.0);
        assert_eq!(library.worlds.len(), 3);
        assert!(library.warnings.is_empty());
        assert!(library.worlds.iter().any(|w| w.location == "1.21.1"));
    }
    #[test]
    fn names_persist_without_changing_game_data() {
        let fixture = Fixture::new();
        let world = fixture.world("saves/town");
        write_label(&world, "我们的生存小镇").unwrap();
        assert_eq!(scan(&fixture.0).worlds[0].name, "我们的生存小镇");
        write_label(&world, "新的中文名称").unwrap();
        assert_eq!(scan(&fixture.0).worlds[0].name, "新的中文名称");
        assert_eq!(
            fs::read(world.join("level.dat")).unwrap(),
            b"original world metadata"
        );
        assert_eq!(
            fs::read(world.join("region/r.0.0.mca")).unwrap(),
            b"original chunks"
        );
    }
    #[test]
    fn imports_twice_without_overwriting_source_or_existing_world() {
        let fixture = Fixture::new();
        let source = fixture.world("download/map");
        let root = fixture.0.join("minecraft");
        import_folder(&root, &source, "小岛").unwrap();
        import_folder(&root, &source, "小岛").unwrap();
        let library = scan(&root);
        assert_eq!(library.worlds.len(), 2);
        assert_ne!(library.worlds[0].id, library.worlds[1].id);
        for world in library.worlds {
            assert_eq!(world.name, "小岛");
            assert_eq!(
                fs::read(root.join(world.id).join("region/r.0.0.mca")).unwrap(),
                b"original chunks"
            );
        }
        assert!(!source.join(LABEL_FILE).exists());
        assert!(source.join("level.dat").exists());
    }
    #[test]
    fn rejects_invalid_names_and_paths() {
        let fixture = Fixture::new();
        fixture.world("saves/town");
        for name in ["", "  ", "bad\nname"] {
            assert!(validate_name(name).is_err());
        }
        assert!(validate_name(&"a".repeat(81)).is_err());
        assert_eq!(validate_name(" 小镇 ").unwrap(), "小镇");
        for id in [
            "../outside",
            "saves/../../outside",
            "C:/outside",
            "/outside",
        ] {
            assert!(resolve_world(&fixture.0, id).is_err());
        }
        assert!(resolve_world(&fixture.0, "saves/town").is_ok());
        assert!(import_folder(
            &fixture.0.join("saves/town/inside"),
            &fixture.0.join("saves/town"),
            "循环"
        )
        .is_err());
        assert!(!fixture.0.join("saves/town/inside").exists());
    }
}
