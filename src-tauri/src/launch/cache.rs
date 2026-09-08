use super::hotset;
use super::profile::LaunchSnapshot;
use super::usn;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, UNIX_EPOCH},
};

pub const CACHE_SCHEMA: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileStamp {
    pub path: String,
    pub size: u64,
    pub modified_ns: u64,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Fingerprint {
    schema: u32,
    base_version: String,
    launch_version: String,
    runtime_options: String,
    java: Option<FileStamp>,
    version_files: Vec<FileStamp>,
    mods: Vec<FileStamp>,
    resourcepacks: Vec<FileStamp>,
    shaderpacks: Vec<FileStamp>,
    libraries_root: Option<FileStamp>,
    assets_root: Option<FileStamp>,
    natives_root: Option<FileStamp>,
    usn: Option<usn::UsnState>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstanceSnapshot {
    pub schema: u32,
    pub generation: u64,
    pub previous_generation: Option<u64>,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub invalidations: Vec<CacheInvalidation>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CacheInvalidation {
    pub category: String,
    pub target: String,
    pub change: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModRecord {
    pub path: String,
    pub size: u64,
    pub mtime: u64,
    pub hash: Option<[u8; 32]>,
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub loader: String,
    pub dependencies: Vec<String>,
    pub icon: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PathRecord {
    stamp: FileStamp,
    hash: Option<[u8; 32]>,
}

pub struct CommitStats {
    pub generation: u64,
    pub mods: Duration,
    pub libraries: Duration,
    pub assets: Duration,
    pub natives: Duration,
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    schema: u32,
    value: T,
}

pub struct CacheCheck {
    pub snapshot: Option<LaunchSnapshot>,
    pub hit_rate: f32,
    fingerprint: Fingerprint,
    previous: Option<Fingerprint>,
    root: PathBuf,
}

fn stamp(root: &Path, path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .min(u64::MAX as u128) as u64;
    Some(FileStamp {
        path: path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/"),
        size: metadata.len(),
        modified_ns,
    })
}

fn scan_files(
    root: &Path,
    directory: &Path,
    recursive: bool,
    extensions: Option<&[&str]>,
) -> Vec<FileStamp> {
    let mut result = Vec::new();
    let mut pending = vec![directory.to_owned()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() && recursive {
                pending.push(path);
            } else if kind.is_file() {
                let allowed = extensions.is_none_or(|values| {
                    path.extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| {
                            values.iter().any(|item| value.eq_ignore_ascii_case(item))
                        })
                });
                if allowed {
                    if let Some(value) = stamp(root, &path) {
                        result.push(value);
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

fn read_cache<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    let envelope: Envelope<T> = bincode::deserialize(&bytes).ok()?;
    (envelope.schema == CACHE_SCHEMA).then_some(envelope.value)
}

fn write_cache<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = bincode::serialize(&Envelope {
        schema: CACHE_SCHEMA,
        value,
    })
    .map_err(|e| e.to_string())?;
    let temp = path.with_extension("bin.tmp");
    let mut file = File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

fn quick_fingerprint(
    root: &Path,
    base: &str,
    launch: &str,
    java: &str,
    memory: u32,
    player: &str,
) -> Fingerprint {
    let versions = root.join("versions");
    let version_paths = [base, launch]
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter_map(|id| stamp(root, &versions.join(id).join(format!("{id}.json"))))
        .collect();
    let mut fingerprint = Fingerprint {
        schema: CACHE_SCHEMA,
        base_version: base.into(),
        launch_version: launch.into(),
        runtime_options: format!("{java}\0{memory}\0{player}"),
        java: stamp(root, Path::new(java)),
        version_files: version_paths,
        mods: scan_files(root, &root.join("game/mods"), false, Some(&["jar", "zip"])),
        resourcepacks: scan_files(
            root,
            &root.join("game/resourcepacks"),
            false,
            Some(&["zip"]),
        ),
        shaderpacks: scan_files(root, &root.join("game/shaderpacks"), false, Some(&["zip"])),
        libraries_root: stamp(root, &root.join("libraries")),
        assets_root: stamp(root, &root.join("assets")),
        natives_root: stamp(root, &versions.join(base).join("natives")),
        usn: None,
    };
    fingerprint.usn = usn::snapshot(root, java);
    fingerprint
}

pub fn check(
    root: &Path,
    base: &str,
    launch: &str,
    java: &str,
    memory: u32,
    player: &str,
) -> CacheCheck {
    let cache_root = root.join(".catcl/cache");
    let previous: Option<Fingerprint> = read_cache(&cache_root.join("fingerprint.bin"));
    let usn_fingerprint = previous.as_ref().and_then(|old| {
        usn::unchanged(root, old.usn.as_ref()).map(|state| {
            let mut current = old.clone();
            current.base_version = base.into();
            current.launch_version = launch.into();
            current.runtime_options = format!("{java}\0{memory}\0{player}");
            current.java = stamp(root, Path::new(java));
            current.usn = Some(state);
            current
        })
    });
    let usn_unchanged = usn_fingerprint.is_some();
    let fingerprint = usn_fingerprint
        .unwrap_or_else(|| quick_fingerprint(root, base, launch, java, memory, player));
    let exact = previous
        .as_ref()
        .is_some_and(|old| launch_critical_equal(old, &fingerprint))
        && (usn_unchanged
            || (validate_path_records(root, &cache_root.join("libraries.bin"))
                && validate_path_records(root, &cache_root.join("assets.bin"))
                && validate_path_records(root, &cache_root.join("natives.bin"))));
    let snapshot = exact
        .then(|| read_cache::<LaunchSnapshot>(&cache_root.join("launch.bin")))
        .flatten()
        .filter(|snapshot| {
            snapshot.schema == CACHE_SCHEMA && Path::new(&snapshot.working_directory).is_dir()
        });
    if usn_unchanged && snapshot.is_some() {
        let _ = write_cache(&cache_root.join("fingerprint.bin"), &fingerprint);
    }
    let full_hit = previous.as_ref().is_some_and(|old| {
        launch_critical_equal(old, &fingerprint)
            && old.resourcepacks == fingerprint.resourcepacks
            && old.shaderpacks == fingerprint.shaderpacks
    });
    CacheCheck {
        snapshot,
        hit_rate: if full_hit {
            1.0
        } else {
            section_hit_rate(previous.as_ref(), &fingerprint)
        },
        fingerprint,
        previous,
        root: cache_root,
    }
}

fn launch_critical_equal(old: &Fingerprint, new: &Fingerprint) -> bool {
    old.schema == new.schema
        && old.base_version == new.base_version
        && old.launch_version == new.launch_version
        && old.runtime_options == new.runtime_options
        && old.java == new.java
        && old.version_files == new.version_files
        && old.mods == new.mods
        && old.libraries_root == new.libraries_root
        && old.assets_root == new.assets_root
        && old.natives_root == new.natives_root
}

fn validate_path_records(root: &Path, cache_file: &Path) -> bool {
    let Some(records) = read_cache::<Vec<PathRecord>>(cache_file) else {
        return false;
    };
    records.into_iter().all(|record| {
        let path = root.join(
            record
                .stamp
                .path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        stamp(root, &path).as_ref() == Some(&record.stamp)
    })
}

fn section_hit_rate(old: Option<&Fingerprint>, new: &Fingerprint) -> f32 {
    let Some(old) = old else { return 0.0 };
    let checks = [
        old.base_version == new.base_version && old.launch_version == new.launch_version,
        old.runtime_options == new.runtime_options,
        old.java == new.java,
        old.version_files == new.version_files,
        old.mods == new.mods,
        old.resourcepacks == new.resourcepacks,
        old.shaderpacks == new.shaderpacks,
        old.libraries_root == new.libraries_root,
        old.assets_root == new.assets_root,
        old.natives_root == new.natives_root,
    ];
    checks.into_iter().filter(|value| *value).count() as f32 / checks.len() as f32
}

fn changes(
    old: Option<&Fingerprint>,
    new: &Fingerprint,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let old_paths: HashMap<_, _> = old
        .into_iter()
        .flat_map(|value| {
            value
                .mods
                .iter()
                .chain(value.resourcepacks.iter())
                .chain(value.shaderpacks.iter())
        })
        .map(|stamp| (stamp.path.clone(), stamp))
        .collect();
    let new_paths: HashMap<_, _> = new
        .mods
        .iter()
        .chain(new.resourcepacks.iter())
        .chain(new.shaderpacks.iter())
        .map(|stamp| (stamp.path.clone(), stamp))
        .collect();
    let added = new_paths
        .keys()
        .filter(|path| !old_paths.contains_key(*path))
        .cloned()
        .collect();
    let removed = old_paths
        .keys()
        .filter(|path| !new_paths.contains_key(*path))
        .cloned()
        .collect();
    let changed = new_paths
        .iter()
        .filter(|(path, stamp)| old_paths.get(*path).is_some_and(|old| *old != **stamp))
        .map(|(path, _)| path.clone())
        .collect();
    (added, changed, removed)
}

fn invalidations(
    added: &[String],
    changed: &[String],
    removed: &[String],
) -> Vec<CacheInvalidation> {
    let mut result = Vec::new();
    for (change, paths) in [("added", added), ("changed", changed), ("removed", removed)] {
        result.extend(paths.iter().map(|path| {
            CacheInvalidation {
                category: if path.starts_with("game/mods/") {
                    "launch-critical"
                } else {
                    "resource-critical"
                }
                .into(),
                target: path.clone(),
                change: change.into(),
            }
        }));
    }
    result
}

fn parse_mod(root: &Path, stamp: &FileStamp, prior: Option<&ModRecord>) -> ModRecord {
    let path = root.join(stamp.path.replace('/', std::path::MAIN_SEPARATOR_STR));
    let mut record = ModRecord {
        path: stamp.path.clone(),
        size: stamp.size,
        mtime: stamp.modified_ns,
        hash: None,
        mod_id: String::new(),
        name: String::new(),
        version: String::new(),
        loader: String::new(),
        dependencies: Vec::new(),
        icon: None,
    };
    if prior.is_some() {
        if let Ok(mut file) = File::open(&path) {
            let mut hasher = blake3::Hasher::new();
            let mut buffer = [0u8; 65536];
            while let Ok(count) = file.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            record.hash = Some(*hasher.finalize().as_bytes());
        }
    }
    let Ok(file) = File::open(path) else {
        return record;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return record;
    };
    if let Ok(entry) = archive.by_name("fabric.mod.json") {
        let mut bytes = Vec::new();
        if entry.take(2 * 1024 * 1024).read_to_end(&mut bytes).is_ok() {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                record.loader = "fabric".into();
                record.mod_id = value["id"].as_str().unwrap_or_default().into();
                record.name = value["name"].as_str().unwrap_or_default().into();
                record.version = value["version"].as_str().unwrap_or_default().into();
                record.icon = value["icon"].as_str().map(str::to_owned);
                if let Some(items) = value["depends"].as_object() {
                    record.dependencies = items.keys().cloned().collect();
                }
            }
        }
        return record;
    }
    for (entry_name, loader) in [
        ("META-INF/neoforge.mods.toml", "neoforge"),
        ("META-INF/mods.toml", "forge"),
    ] {
        if let Ok(entry) = archive.by_name(entry_name) {
            let mut source = String::new();
            let _ = entry.take(2 * 1024 * 1024).read_to_string(&mut source);
            record.loader = loader.into();
            for line in source.lines().map(str::trim) {
                let value = |prefix: &str| {
                    line.strip_prefix(prefix)?
                        .split_once('=')?
                        .1
                        .trim()
                        .trim_matches('"')
                        .to_owned()
                        .into()
                };
                if record.mod_id.is_empty() {
                    if let Some(found) = value("modId") {
                        record.mod_id = found;
                    }
                }
                if record.name.is_empty() {
                    if let Some(found) = value("displayName") {
                        record.name = found;
                    }
                }
                if record.version.is_empty() {
                    if let Some(found) = value("version") {
                        record.version = found;
                    }
                }
            }
            break;
        }
    }
    record
}

pub fn commit(
    check: CacheCheck,
    mut snapshot: LaunchSnapshot,
    classpath: &[PathBuf],
) -> Result<CommitStats, String> {
    std::fs::create_dir_all(&check.root).map_err(|e| e.to_string())?;
    let old_snapshot: Option<InstanceSnapshot> = read_cache(&check.root.join("instance.bin"));
    let generation = old_snapshot.map_or(1, |value| value.generation + 1);
    let (added, changed, removed) = changes(check.previous.as_ref(), &check.fingerprint);
    let cache_invalidations = invalidations(&added, &changed, &removed);
    let instance = InstanceSnapshot {
        schema: CACHE_SCHEMA,
        generation,
        previous_generation: generation.checked_sub(1).filter(|value| *value > 0),
        added,
        changed,
        removed,
        invalidations: cache_invalidations,
    };
    let timer = Instant::now();
    let prior_mods: Vec<ModRecord> = read_cache(&check.root.join("mods.bin")).unwrap_or_default();
    let prior: HashMap<_, _> = prior_mods
        .iter()
        .map(|item| (item.path.as_str(), item))
        .collect();
    let mods: Vec<_> = check
        .fingerprint
        .mods
        .iter()
        .map(|item| match prior.get(item.path.as_str()) {
            Some(cached) if cached.size == item.size && cached.mtime == item.modified_ns => {
                (*cached).clone()
            }
            cached => parse_mod(
                check
                    .root
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or(&check.root),
                item,
                cached.copied(),
            ),
        })
        .collect();
    let mods_elapsed = timer.elapsed();
    let timer = Instant::now();
    let libraries: Vec<PathRecord> = classpath
        .iter()
        .filter_map(|path| {
            stamp(
                check
                    .root
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or(&check.root),
                path,
            )
        })
        .map(|stamp| PathRecord { stamp, hash: None })
        .collect();
    let libraries_elapsed = timer.elapsed();
    let timer = Instant::now();
    let assets = check
        .fingerprint
        .assets_root
        .clone()
        .into_iter()
        .map(|stamp| PathRecord { stamp, hash: None })
        .collect::<Vec<_>>();
    let assets_elapsed = timer.elapsed();
    let timer = Instant::now();
    let natives = check
        .fingerprint
        .natives_root
        .clone()
        .into_iter()
        .map(|stamp| PathRecord { stamp, hash: None })
        .collect::<Vec<_>>();
    let natives_elapsed = timer.elapsed();
    snapshot.schema = CACHE_SCHEMA;
    snapshot.generation = generation;
    write_cache(&check.root.join("mods.bin"), &mods)?;
    write_cache(&check.root.join("libraries.bin"), &libraries)?;
    write_cache(&check.root.join("assets.bin"), &assets)?;
    write_cache(&check.root.join("natives.bin"), &natives)?;
    write_cache(&check.root.join("instance.bin"), &instance)?;
    write_cache(&check.root.join("launch.bin"), &snapshot)?;
    write_cache(&check.root.join("fingerprint.bin"), &check.fingerprint)?;
    let instance_root = check
        .root
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&check.root);
    let mut hot_paths = classpath.to_vec();
    hot_paths.extend(
        check
            .fingerprint
            .mods
            .iter()
            .map(|item| instance_root.join(item.path.replace('/', std::path::MAIN_SEPARATOR_STR))),
    );
    let _ = hotset::learn(instance_root, generation, &hot_paths);
    Ok(CommitStats {
        generation,
        mods: mods_elapsed,
        libraries: libraries_elapsed,
        assets: assets_elapsed,
        natives: natives_elapsed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_instance(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "catcl-cache-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("game/mods")).unwrap();
        std::fs::create_dir_all(root.join("game/config")).unwrap();
        std::fs::create_dir_all(root.join("versions/1.20.1")).unwrap();
        std::fs::write(root.join("versions/1.20.1/1.20.1.json"), b"{}").unwrap();
        root
    }

    fn snapshot(root: &Path) -> LaunchSnapshot {
        LaunchSnapshot {
            schema: CACHE_SCHEMA,
            generation: 0,
            java_path: "java".into(),
            working_directory: root.join("game").to_string_lossy().into_owned(),
            arguments: vec!["-version".into()],
            classpath_entries: 0,
            main_class: "example.Main".into(),
        }
    }

    #[test]
    fn exact_fingerprint_reuses_launch_snapshot() {
        let root = temp_instance("hit");
        let initial = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        commit(initial, snapshot(&root), &[]).unwrap();
        let hit = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        assert!(hit.snapshot.is_some());
        assert_eq!(hit.hit_rate, 1.0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_launch_cache_falls_back_to_full_launch() {
        let root = temp_instance("corrupt");
        let initial = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        commit(initial, snapshot(&root), &[]).unwrap();
        std::fs::write(root.join(".catcl/cache/launch.bin"), b"broken").unwrap();
        let fallback = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        assert!(fallback.snapshot.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_mod_change_is_reported_incrementally() {
        let root = temp_instance("incremental");
        let first = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        commit(first, snapshot(&root), &[]).unwrap();
        std::fs::write(root.join("game/mods/added.jar"), b"not-a-real-jar").unwrap();
        let second = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        commit(second, snapshot(&root), &[]).unwrap();
        let state: InstanceSnapshot = read_cache(&root.join(".catcl/cache/instance.bin")).unwrap();
        assert_eq!(state.generation, 2);
        assert_eq!(state.added, vec!["game/mods/added.jar"]);
        assert!(state.changed.is_empty());
        assert!(state.removed.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_mutable_files_do_not_invalidate_launch() {
        let root = temp_instance("runtime-mutable");
        let initial = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        commit(initial, snapshot(&root), &[]).unwrap();
        std::fs::write(root.join("game/options.txt"), b"changed=true").unwrap();
        std::fs::write(root.join("game/config/runtime.toml"), b"value=2").unwrap();
        let hit = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        assert!(hit.snapshot.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resource_pack_change_preserves_launch_snapshot() {
        let root = temp_instance("resource-only");
        std::fs::create_dir_all(root.join("game/resourcepacks")).unwrap();
        let initial = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        commit(initial, snapshot(&root), &[]).unwrap();
        std::fs::write(root.join("game/resourcepacks/new.zip"), b"pack").unwrap();
        let hit = check(&root, "1.20.1", "1.20.1", "java", 4096, "Player");
        assert!(hit.snapshot.is_some());
        assert!(hit.hit_rate < 1.0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
