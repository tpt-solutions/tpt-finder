// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Popup overlay window control (summon/dismiss via global hotkey).

use tauri::{AppHandle, Emitter, Manager};

pub const POPUP_LABEL: &str = "popup";

/// Show + focus the popup, centering it and notifying the frontend.
pub fn show_popup(app: &AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window(POPUP_LABEL)
        .ok_or_else(|| "popup window not found".to_string())?;
    if win.is_visible().unwrap_or(false) {
        let _ = win.set_focus();
        let _ = app.emit("popup://show", ());
        return Ok(());
    }
    let _ = win.center();
    win.show().map_err(|e| e.to_string())?;
    let _ = win.set_focus();
    let _ = app.emit("popup://show", ());
    Ok(())
}

pub fn hide_popup(app: &AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window(POPUP_LABEL)
        .ok_or_else(|| "popup window not found".to_string())?;
    win.hide().map_err(|e| e.to_string())?;
    let _ = app.emit("popup://hide", ());
    Ok(())
}

/// Returns true if the popup is visible after the call.
pub fn toggle_popup(app: &AppHandle) -> Result<bool, String> {
    let win = app
        .get_webview_window(POPUP_LABEL)
        .ok_or_else(|| "popup window not found".to_string())?;
    if win.is_visible().unwrap_or(false) {
        hide_popup(app)?;
        Ok(false)
    } else {
        show_popup(app)?;
        Ok(true)
    }
}
