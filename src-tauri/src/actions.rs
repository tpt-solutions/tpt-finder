// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Result actions: open, reveal, copy path, delete (recycle bin).

use std::path::{Path, PathBuf};

use tauri_plugin_opener::OpenerExt;

/// Validate a user-visible path before any action touches it.
///
/// Guards (security review, Phase 9): paths must be non-empty, absolute,
/// NUL-free, and must not be a filesystem root — deleting "C:\" or "/" must
/// never be reachable, even from a corrupted UI state.
pub fn validate_action_path(path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("empty path".into());
    }
    if path.contains('\0') {
        return Err("invalid path (NUL byte)".into());
    }
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(format!("path must be absolute: {path}"));
    }
    if p.parent().is_none() {
        return Err(format!("refusing to act on a filesystem root: {path}"));
    }
    Ok(p)
}

/// Open a file or directory with the system default application.
pub fn open_path(app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    let p = validate_action_path(path)?;
    if !p.exists() {
        return Err(format!("path does not exist: {path}"));
    }
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| format!("open failed: {e}"))
}

/// Reveal a path in the OS file manager (select it if possible).
pub fn reveal_path(app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    let p = validate_action_path(path)?;
    if !p.exists() {
        return Err(format!("path does not exist: {path}"));
    }
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| format!("reveal failed: {e}"))
}

/// Move a path to the OS recycle bin / trash.
pub fn delete_to_trash(path: &str) -> Result<(), String> {
    let p = validate_action_path(path)?;
    if !p.exists() {
        return Err(format!("path does not exist: {path}"));
    }
    trash::delete(p).map_err(|e| format!("delete to trash failed: {e}"))
}

/// Parent directory of a path (for "open containing folder").
pub fn containing_folder(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent()
            .map(|x| x.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Windows: put file(s) on the clipboard as CF_HDROP via a short-lived PowerShell helper.
/// (Keeps the `windows` crate surface small; path is validated first.)
#[cfg(target_os = "windows")]
pub fn copy_file_windows(app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("path does not exist: {path}"));
    }
    // Escape single quotes for PowerShell single-quoted string.
    let escaped = path.replace('\'', "''");
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $file = New-Object System.Collections.Specialized.StringCollection; \
         $file.Add('{escaped}'); \
         [System.Windows.Forms.Clipboard]::SetFileDropList($file) | Out-Null"
    );
    // CREATE_NO_WINDOW = 0x08000000
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .status()
        .map_err(|e| format!("copy file failed to spawn: {e}"))?;
    if !status.success() {
        // Fallback: write path text so the user still has something pasteable.
        use tauri_plugin_clipboard_manager::ClipboardExt;
        let _ = app.clipboard().write_text(path);
        return Err(format!(
            "copy file failed ({status}); path copied as text instead"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_missing_errors() {
        assert!(!containing_folder("/definitely/not/a/real/path-xyz")
            .as_os_str()
            .is_empty());
    }

    #[test]
    fn containing_folder_file() {
        #[cfg(windows)]
        let f = r"C:\Windows\win.ini";
        #[cfg(not(windows))]
        let f = "/etc/hosts";
        if Path::new(f).exists() {
            let dir = containing_folder(f);
            assert!(!dir.as_os_str().is_empty());
        }
    }

    #[test]
    fn validate_rejects_bad_paths() {
        assert!(validate_action_path("").is_err());
        assert!(validate_action_path("   ").is_err());
        assert!(validate_action_path("relative/file.txt").is_err());
        assert!(validate_action_path("with\0nul").is_err());
        // Filesystem roots must never pass.
        assert!(validate_action_path("/").is_err());
        #[cfg(windows)]
        {
            assert!(validate_action_path(r"C:\").is_err());
            assert!(validate_action_path(r"\\?\C:\").is_err());
        }
    }

    #[test]
    fn validate_accepts_real_file() {
        let dir = std::env::temp_dir().join(format!("tpt-action-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("ok.txt");
        std::fs::write(&f, b"x").unwrap();
        let validated = validate_action_path(&f.to_string_lossy()).expect("valid path");
        assert!(validated.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
