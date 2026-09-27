// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Windows platform helpers: volume enumeration, file IDs, USN Journal watch.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDiskFreeSpaceW,
    GetFileInformationByHandle, GetVolumeInformationW, GetVolumePathNamesForVolumeNameW,
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
};
use windows::Win32::System::IO::DeviceIoControl;

use super::{BackendError, FsChange, LiveWatch};
use crate::index::model::{FileEntry, VolumeInfo};

pub const PLATFORM_BACKEND_NAME: &str = "walk+usn";

const USN_REASON_FILE_CREATE: u32 = 0x0000_0001;
const USN_REASON_FILE_DELETE: u32 = 0x0000_0100;
const USN_REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;
const USN_REASON_BASIC_INFO_CHANGE: u32 = 0x0000_4000;
const USN_REASON_CLOSE: u32 = 0x8000_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// Enumerate volumes via FindFirstVolume / GetVolumeInformationW.
pub fn platform_volumes() -> Result<Vec<VolumeInfo>, BackendError> {
    let mut out = Vec::new();
    let mut name_buf = [0u16; 512];
    let h = unsafe { FindFirstVolumeW(&mut name_buf) }
        .map_err(|e| BackendError::Other(format!("FindFirstVolumeW failed: {e}")))?;

    loop {
        let len = name_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(name_buf.len());
        let vol_path = String::from_utf16_lossy(&name_buf[..len]);
        if let Some(info) = volume_info_from_nt_path(&vol_path) {
            out.push(info);
        }
        name_buf = [0u16; 512];
        if unsafe { FindNextVolumeW(h, &mut name_buf) }.is_err() {
            break;
        }
    }
    let _ = unsafe { FindVolumeClose(h) };

    if out.is_empty() {
        out.push(VolumeInfo {
            id: 1,
            root: "C:\\".into(),
            fs: "NTFS".into(),
            total_bytes: 0,
            free_bytes: 0,
            live_updates: true,
        });
    }
    Ok(out)
}

fn volume_info_from_nt_path(vol_path: &str) -> Option<VolumeInfo> {
    let wide: Vec<u16> = vol_path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut path_buf = [0u16; 512];
    let mut return_len = 0u32;
    unsafe {
        GetVolumePathNamesForVolumeNameW(
            PCWSTR(wide.as_ptr()),
            Some(&mut path_buf),
            &mut return_len,
        )
    }
    .ok()?;
    if return_len == 0 {
        return None;
    }
    let paths_str = String::from_utf16_lossy(&path_buf[..return_len as usize]);
    let root = paths_str.split('\0').find(|s| !s.is_empty())?.to_string();
    if root.is_empty() {
        return None;
    }

    let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
    let mut fs_buf = [0u16; 64];
    let mut serial = 0u32;
    let fs_ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(root_wide.as_ptr()),
            None,
            Some(&mut serial),
            None,
            None,
            Some(&mut fs_buf),
        )
    };
    let fs = if fs_ok.is_ok() {
        String::from_utf16_lossy(
            &fs_buf[..fs_buf.iter().position(|&c| c == 0).unwrap_or(fs_buf.len())],
        )
    } else {
        "unknown".into()
    };
    let live = fs.eq_ignore_ascii_case("NTFS");

    let mut sectors_per_cluster = 0u32;
    let mut bytes_per_sector = 0u32;
    let mut free_clusters = 0u32;
    let mut total_clusters = 0u32;
    let mut total_bytes = 0u64;
    let mut free_bytes = 0u64;
    if unsafe {
        GetDiskFreeSpaceW(
            PCWSTR(root_wide.as_ptr()),
            Some(&mut sectors_per_cluster),
            Some(&mut bytes_per_sector),
            Some(&mut free_clusters),
            Some(&mut total_clusters),
        )
    }
    .is_ok()
    {
        let cluster = u64::from(sectors_per_cluster) * u64::from(bytes_per_sector);
        total_bytes = u64::from(total_clusters) * cluster;
        free_bytes = u64::from(free_clusters) * cluster;
    }

    let id = if serial != 0 {
        serial.max(1)
    } else {
        stable_volume_id(&root)
    };

    Some(VolumeInfo {
        id,
        root,
        fs,
        total_bytes,
        free_bytes,
        live_updates: live,
    })
}

fn stable_volume_id(root: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in root.to_ascii_lowercase().bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

/// Windows file reference number (FRN) via GetFileInformationByHandle.
pub fn platform_file_id(path: &Path) -> Option<u64> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
        .ok()?;
    let handle = HANDLE(file.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut info) }.ok()?;
    let frn = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some(frn)
}

fn open_volume_handle(volume_root: &str) -> Result<HANDLE, BackendError> {
    let trimmed = volume_root.trim_end_matches(['\\', '/']);
    if !(trimmed.len() == 2 && trimmed.ends_with(':')) {
        return Err(BackendError::Unsupported(format!(
            "cannot open volume handle for {volume_root}"
        )));
    }
    let drive = format!("\\\\.\\{}", &trimmed[..2]);
    let wide: Vec<u16> = drive.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| BackendError::Other(format!("CreateFileW volume {drive}: {e}")))?;
    if handle == INVALID_HANDLE_VALUE {
        return Err(BackendError::Other(format!("invalid handle for {drive}")));
    }
    Ok(handle)
}

fn close_handle(h: HANDLE) {
    if h != INVALID_HANDLE_VALUE {
        let _ = unsafe { CloseHandle(h) };
    }
}

fn query_journal(handle: HANDLE) -> Result<USN_JOURNAL_DATA_V0, BackendError> {
    let mut data = USN_JOURNAL_DATA_V0::default();
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(&mut data as *mut _ as *mut _),
            std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|e| BackendError::Other(format!("FSCTL_QUERY_USN_JOURNAL: {e}")))?;
    Ok(data)
}

fn read_usn_batch(
    handle: HANDLE,
    start_usn: i64,
    journal_id: u64,
    buf: &mut [u8],
) -> Result<(Vec<UsnRec>, i64), BackendError> {
    let data = READ_USN_JOURNAL_DATA_V0 {
        StartUsn: start_usn,
        ReasonMask: u32::MAX,
        ReturnOnlyOnClose: 0,
        Timeout: 0,
        BytesToWaitFor: 0,
        UsnJournalID: journal_id,
    };
    let mut returned = 0u32;
    let result = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_READ_USN_JOURNAL,
            Some(&data as *const _ as *const _),
            std::mem::size_of_val(&data) as u32,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            buf.len() as u32,
            Some(&mut returned),
            None,
        )
    };
    if let Err(e) = result {
        // ERROR_HANDLE_EOF (38) → HRESULT 0x80070026
        if (e.code().0 as u32) == 0x8007_0026 {
            return Ok((Vec::new(), start_usn));
        }
        return Err(BackendError::Other(format!("FSCTL_READ_USN_JOURNAL: {e}")));
    }
    if returned < 8 {
        return Ok((Vec::new(), start_usn));
    }
    let next = i64::from_le_bytes(buf[0..8].try_into().unwrap());
    let mut recs = Vec::new();
    let mut offset = 8usize;
    while offset + 8 <= returned as usize {
        let len = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        if len < 64 || offset + len > returned as usize {
            break;
        }
        if let Some(rec) = parse_usn_record(&buf[offset..offset + len]) {
            recs.push(rec);
        }
        offset += len;
    }
    Ok((recs, next))
}

#[derive(Debug, Clone)]
struct UsnRec {
    frn: u64,
    parent_frn: u64,
    usn: i64,
    reason: u32,
    file_attributes: u32,
    name: String,
}

fn parse_usn_record(buf: &[u8]) -> Option<UsnRec> {
    if buf.len() < 64 {
        return None;
    }
    let length = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    let major = u16::from_le_bytes(buf[4..6].try_into().unwrap());
    if major < 2 {
        return None;
    }
    let frn = u64::from_le_bytes(buf[8..16].try_into().unwrap());
    let parent_frn = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let usn = i64::from_le_bytes(buf[24..32].try_into().unwrap());
    let reason = u32::from_le_bytes(buf[40..44].try_into().unwrap());
    let file_attributes = u32::from_le_bytes(buf[52..56].try_into().unwrap());
    let name_len = u32::from_le_bytes(buf[56..60].try_into().unwrap()) as usize;
    let name_off = u32::from_le_bytes(buf[60..64].try_into().unwrap()) as usize;
    if name_off + name_len > length.min(buf.len()) {
        return None;
    }
    let name_bytes = &buf[name_off..name_off + name_len];
    let name = String::from_utf16_lossy(
        &name_bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
    );
    Some(UsnRec {
        frn,
        parent_frn,
        usn,
        reason,
        file_attributes,
        name,
    })
}

struct UsnWatchHandle {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    /// Raw HANDLE value as isize so the type is Send (HANDLE is not Send).
    handle_isize: isize,
    last_usn: Arc<AtomicU64>,
}

impl LiveWatch for UsnWatchHandle {
    fn checkpoint(&self) -> Option<u64> {
        Some(self.last_usn.load(Ordering::Relaxed))
    }
}

impl Drop for UsnWatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        close_handle(HANDLE(self.handle_isize as *mut core::ffi::c_void));
    }
}

/// Start a USN Journal tail for `volume`. Full scan is separate (walk backend);
/// this only emits incremental FsChange events.
pub fn start_usn_watch(
    volume: &VolumeInfo,
    resume_from_usn: Option<u64>,
) -> Result<(std::sync::mpsc::Receiver<FsChange>, Box<dyn LiveWatch>), BackendError> {
    if !volume.live_updates {
        return Err(BackendError::Unsupported(format!(
            "live USN updates not available on {} ({})",
            volume.root, volume.fs
        )));
    }
    let handle = open_volume_handle(&volume.root)?;
    let journal = match query_journal(handle) {
        Ok(j) => j,
        Err(e) => {
            close_handle(handle);
            return Err(e);
        }
    };

    let start = resume_from_usn.map(|v| v as i64).unwrap_or(journal.NextUsn);
    if start < journal.LowestValidUsn {
        close_handle(handle);
        return Err(BackendError::JournalWrap);
    }

    let (tx, rx) = std::sync::mpsc::channel::<FsChange>();
    let stop = Arc::new(AtomicBool::new(false));
    let last_usn = Arc::new(AtomicU64::new(start.max(0) as u64));

    let dir_paths: Arc<Mutex<HashMap<u64, String>>> = Arc::new(Mutex::new(HashMap::new()));
    {
        let mut map = dir_paths.lock().unwrap();
        if let Some(root_frn) = platform_file_id(Path::new(&volume.root)) {
            map.insert(root_frn, volume.root.clone());
        }
    }

    let stop2 = Arc::clone(&stop);
    let last2 = Arc::clone(&last_usn);
    let dir_paths2 = Arc::clone(&dir_paths);
    let root = volume.root.clone();
    let vol_id = volume.id;
    let mut cur_usn = start;
    let journal_id = journal.UsnJournalID;
    let handle_raw = handle.0 as isize;

    let join = std::thread::Builder::new()
        .name("usn-watch".into())
        .spawn(move || {
            let thread_handle = HANDLE(handle_raw as *mut core::ffi::c_void);
            let mut buf = vec![0u8; 1024 * 1024];
            while !stop2.load(Ordering::Relaxed) {
                match read_usn_batch(thread_handle, cur_usn, journal_id, &mut buf) {
                    Ok((recs, next)) => {
                        if recs.is_empty() {
                            if next > cur_usn {
                                cur_usn = next;
                                last2.store(cur_usn.max(0) as u64, Ordering::Relaxed);
                            }
                            std::thread::sleep(std::time::Duration::from_millis(250));
                            continue;
                        }
                        for rec in recs {
                            if rec.usn > cur_usn {
                                cur_usn = rec.usn;
                            }
                            process_record(&rec, &root, vol_id, &dir_paths2, &tx);
                        }
                        last2.store(cur_usn.max(0) as u64, Ordering::Relaxed);
                        if next > cur_usn {
                            cur_usn = next;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(FsChange::Removed {
                            path: format!("__usn_error__:{e}"),
                            is_dir: false,
                        });
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            }
        })
        .map_err(|e| BackendError::Other(format!("spawn usn-watch: {e}")))?;

    let boxed = Box::new(UsnWatchHandle {
        stop,
        join: Some(join),
        handle_isize: handle.0 as isize,
        last_usn,
    });
    Ok((rx, boxed))
}

fn resolve_path(
    frn: u64,
    parent_frn: u64,
    name: &str,
    is_new_dir: bool,
    dir_paths: &Mutex<HashMap<u64, String>>,
    volume_root: &str,
) -> Option<String> {
    let mut map = dir_paths.lock().unwrap();
    if is_new_dir {
        if let Some(parent) = map.get(&parent_frn) {
            let mut p = parent.clone();
            if !p.ends_with('\\') && !p.ends_with('/') {
                p.push('\\');
            }
            p.push_str(name);
            map.insert(frn, p.clone());
            return Some(p);
        }
        return None;
    }
    if let Some(parent) = map.get(&parent_frn) {
        let mut p = parent.clone();
        if !p.ends_with('\\') && !p.ends_with('/') {
            p.push('\\');
        }
        p.push_str(name);
        return Some(p);
    }
    let _ = volume_root;
    None
}

fn process_record(
    rec: &UsnRec,
    volume_root: &str,
    vol_id: u32,
    dir_paths: &Mutex<HashMap<u64, String>>,
    tx: &Sender<FsChange>,
) {
    let is_create = rec.reason & USN_REASON_FILE_CREATE != 0;
    let is_delete = rec.reason & USN_REASON_FILE_DELETE != 0;
    let is_rename = rec.reason & USN_REASON_RENAME_NEW_NAME != 0;
    let is_info = rec.reason & USN_REASON_BASIC_INFO_CHANGE != 0;
    let _ = rec.reason & USN_REASON_CLOSE != 0;

    if is_delete {
        if let Some(path) = resolve_path(
            rec.frn,
            rec.parent_frn,
            &rec.name,
            false,
            dir_paths,
            volume_root,
        ) {
            let is_dir = rec.file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            if is_dir {
                dir_paths.lock().unwrap().remove(&rec.frn);
            }
            let _ = tx.send(FsChange::Removed { path, is_dir });
        }
        return;
    }

    let is_dir = rec.file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    let Some(path) = resolve_path(
        rec.frn,
        rec.parent_frn,
        &rec.name,
        is_dir && (is_create || is_rename),
        dir_paths,
        volume_root,
    ) else {
        return;
    };

    if is_create || is_rename || is_info {
        let meta = std::fs::metadata(&path).ok();
        let size = meta
            .as_ref()
            .map(|m| if is_dir { 0 } else { m.len() })
            .unwrap_or(0);
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let created = meta
            .as_ref()
            .and_then(|m| m.created().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let accessed = meta
            .as_ref()
            .and_then(|m| m.accessed().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let entry = FileEntry::from_meta(
            0,
            path,
            size,
            created,
            modified,
            accessed,
            rec.file_attributes,
            vol_id,
            is_dir,
            rec.frn,
        );
        if is_create || is_rename {
            let _ = tx.send(FsChange::Created(entry));
        } else {
            let _ = tx.send(FsChange::Updated(entry));
        }
    }
}
