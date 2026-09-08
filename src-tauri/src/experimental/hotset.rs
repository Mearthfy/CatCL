use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA: u32 = 1;
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct HotSet {
    schema: u32,
    generation: u64,
    entries: Vec<HotEntry>,
}

#[derive(Serialize, Deserialize)]
struct HotEntry {
    path: PathBuf,
    size: u64,
    score: u32,
    last_seen_generation: u64,
    estimated_access_phase: String,
}

fn cache_path(root: &Path) -> PathBuf {
    root.join(".catcl/cache/hotset.bin")
}

pub fn learn(root: &Path, generation: u64, paths: &[PathBuf]) -> Result<(), String> {
    let mut entries = paths
        .iter()
        .filter_map(|path| {
            let size = std::fs::metadata(path).ok()?.len();
            let text = path.to_string_lossy().to_ascii_lowercase();
            let (score, phase) = if text.contains("fabric-loader") || text.contains("forge") {
                (100, "Loader")
            } else if text.contains("mixin") {
                (90, "Mixin")
            } else if text.contains("\\mods\\") || text.contains("/mods/") {
                (70, "Mod Init")
            } else {
                (80, "Class Loading")
            };
            Some(HotEntry {
                path: path.clone(),
                size,
                score,
                last_seen_generation: generation,
                estimated_access_phase: phase.into(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| (std::cmp::Reverse(entry.score), entry.size));
    entries.dedup_by(|a, b| a.path == b.path);
    let bytes = bincode::serialize(&HotSet {
        schema: SCHEMA,
        generation,
        entries,
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(cache_path(root), bytes).map_err(|e| e.to_string())
}

pub fn available(root: &Path) -> bool {
    cache_path(root).is_file() && !root.join(".catcl/hotset-disabled").exists()
}

pub fn start(root: PathBuf, generation: u64, limit_mb: u64) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
            };
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
        let Ok(bytes) = std::fs::read(cache_path(&root)) else {
            return;
        };
        let Ok(hotset) = bincode::deserialize::<HotSet>(&bytes) else {
            return;
        };
        if hotset.schema != SCHEMA || hotset.generation != generation {
            return;
        }
        let mut remaining = limit_mb.saturating_mul(1024 * 1024);
        let mut buffer = vec![0u8; 1024 * 1024];
        for entry in hotset.entries {
            if remaining == 0 {
                break;
            }
            let allowance = entry.size.min(MAX_ENTRY_BYTES).min(remaining);
            let Ok(mut file) = File::open(entry.path) else {
                continue;
            };
            let mut read = 0u64;
            while read < allowance {
                let wanted = (allowance - read).min(buffer.len() as u64) as usize;
                let Ok(count) = file.read(&mut buffer[..wanted]) else {
                    break;
                };
                if count == 0 {
                    break;
                }
                read += count as u64;
                std::thread::sleep(Duration::from_millis(2));
            }
            remaining = remaining.saturating_sub(read);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hotset_is_generation_scoped_and_bounded() {
        let root = std::env::temp_dir().join(format!("catcl-hotset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".catcl/cache")).unwrap();
        let jar = root.join("fabric-loader.jar");
        std::fs::write(&jar, b"loader").unwrap();
        learn(&root, 7, &[jar]).unwrap();
        let data: HotSet =
            bincode::deserialize(&std::fs::read(cache_path(&root)).unwrap()).unwrap();
        assert_eq!(data.generation, 7);
        assert_eq!(data.entries[0].score, 100);
        std::fs::remove_dir_all(root).unwrap();
    }
}
