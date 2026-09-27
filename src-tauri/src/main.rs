// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tpt_finder_lib::run()
}
