use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UsnState {
    volume: String,
    volume_serial: u32,
    journal_id: u64,
    next_usn: i64,
    watched_ids: Vec<u64>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::{collections::HashSet, ffi::c_void, mem, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::IO::DeviceIoControl,
    };

    const FSCTL_QUERY_USN_JOURNAL: u32 = 0x0009_00f4;
    const FSCTL_READ_USN_JOURNAL: u32 = 0x0009_00bb;
    const REASON_ANY: u32 = 0xffff_ffff;

    #[repr(C)]
    #[derive(Default)]
    struct JournalData {
        journal_id: u64,
        first_usn: i64,
        next_usn: i64,
        lowest_valid_usn: i64,
        max_usn: i64,
        maximum_size: u64,
        allocation_delta: u64,
    }

    #[repr(C)]
    struct ReadData {
        start_usn: i64,
        reason_mask: u32,
        return_only_on_close: u32,
        timeout: u64,
        bytes_to_wait_for: u64,
        journal_id: u64,
    }

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn open(path: &Path, directory: bool) -> Option<Handle> {
        let flags = FILE_ATTRIBUTE_NORMAL
            | if directory {
                FILE_FLAG_BACKUP_SEMANTICS
            } else {
                0
            };
        let handle = unsafe {
            CreateFileW(
                wide(path.as_os_str()).as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                flags,
                ptr::null_mut(),
            )
        };
        (handle != INVALID_HANDLE_VALUE).then_some(Handle(handle))
    }

    fn file_id(path: &Path) -> Option<(u32, u64)> {
        let handle = open(path, path.is_dir())?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { mem::zeroed() };
        (unsafe { GetFileInformationByHandle(handle.0, &mut info) } != 0).then(|| {
            (
                info.dwVolumeSerialNumber,
                ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            )
        })
    }

    fn volume_path(root: &Path) -> Option<(String, PathBuf)> {
        let prefix = root
            .components()
            .next()?
            .as_os_str()
            .to_string_lossy()
            .to_string();
        let drive = prefix.trim_end_matches(['\\', '/']);
        (drive.len() == 2 && drive.ends_with(':'))
            .then(|| (drive.into(), PathBuf::from(format!(r"\\.\{drive}"))))
    }

    fn journal(handle: HANDLE) -> Option<JournalData> {
        let mut data: JournalData = Default::default();
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_QUERY_USN_JOURNAL,
                ptr::null(),
                0,
                &mut data as *mut _ as *mut c_void,
                mem::size_of::<JournalData>() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        (ok != 0).then_some(data)
    }

    fn managed_paths(root: &Path, java: &str) -> Vec<PathBuf> {
        let mut roots = vec![
            root.join("game/mods"),
            root.join("libraries"),
            root.join("versions"),
            root.join("assets"),
            root.join("game/resourcepacks"),
            root.join("game/shaderpacks"),
        ];
        let java = PathBuf::from(java);
        if java.is_absolute() {
            roots.push(java);
        }
        roots
    }

    fn collect_ids(paths: Vec<PathBuf>, serial: u32) -> Vec<u64> {
        let mut ids = HashSet::new();
        let mut pending = paths;
        while let Some(path) = pending.pop() {
            if let Some((found_serial, id)) = file_id(&path) {
                if found_serial == serial {
                    ids.insert(id);
                }
            }
            if path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let child = entry.path();
                        if child.is_dir() {
                            pending.push(child);
                        } else if let Some((found_serial, id)) = file_id(&child) {
                            if found_serial == serial {
                                ids.insert(id);
                            }
                        }
                    }
                }
            }
        }
        let mut values: Vec<_> = ids.into_iter().collect();
        values.sort_unstable();
        values
    }

    pub fn snapshot(root: &Path, java: &str) -> Option<UsnState> {
        let (volume, device) = volume_path(root)?;
        let handle = open(&device, false)?;
        let journal = journal(handle.0)?;
        let (serial, _) = file_id(root)?;
        Some(UsnState {
            volume,
            volume_serial: serial,
            journal_id: journal.journal_id,
            next_usn: journal.next_usn,
            watched_ids: collect_ids(managed_paths(root, java), serial),
        })
    }

    pub fn unchanged(root: &Path, old: Option<&UsnState>) -> Option<UsnState> {
        let old = old?;
        let (volume, device) = volume_path(root)?;
        if volume != old.volume {
            return None;
        }
        let handle = open(&device, false)?;
        let journal = journal(handle.0)?;
        if journal.journal_id != old.journal_id
            || old.next_usn < journal.lowest_valid_usn
            || old.next_usn > journal.next_usn
        {
            return None;
        }
        let watched: HashSet<u64> = old.watched_ids.iter().copied().collect();
        let mut position = old.next_usn;
        while position < journal.next_usn {
            let previous_position = position;
            let input = ReadData {
                start_usn: position,
                reason_mask: REASON_ANY,
                return_only_on_close: 0,
                timeout: 0,
                bytes_to_wait_for: 0,
                journal_id: old.journal_id,
            };
            let mut buffer = vec![0u8; 1024 * 1024];
            let mut returned = 0u32;
            let ok = unsafe {
                DeviceIoControl(
                    handle.0,
                    FSCTL_READ_USN_JOURNAL,
                    &input as *const _ as *const c_void,
                    mem::size_of::<ReadData>() as u32,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len() as u32,
                    &mut returned,
                    ptr::null_mut(),
                )
            };
            if ok == 0 || returned < 8 {
                return None;
            }
            position = i64::from_le_bytes(buffer[0..8].try_into().ok()?);
            let mut offset = 8usize;
            while offset + 32 <= returned as usize {
                let length =
                    u32::from_le_bytes(buffer[offset..offset + 4].try_into().ok()?) as usize;
                if length < 32 || offset + length > returned as usize {
                    return None;
                }
                let file_id = u64::from_le_bytes(buffer[offset + 8..offset + 16].try_into().ok()?);
                let parent_id =
                    u64::from_le_bytes(buffer[offset + 16..offset + 24].try_into().ok()?);
                if watched.contains(&file_id) || watched.contains(&parent_id) {
                    return None;
                }
                offset += length;
            }
            if position <= previous_position {
                break;
            }
        }
        let mut state = old.clone();
        state.next_usn = journal.next_usn;
        Some(state)
    }
}

#[cfg(windows)]
pub use imp::{snapshot, unchanged};

#[cfg(not(windows))]
pub fn snapshot(_: &Path, _: &str) -> Option<UsnState> {
    None
}
#[cfg(not(windows))]
pub fn unchanged(_: &Path, _: Option<&UsnState>) -> Option<UsnState> {
    None
}
