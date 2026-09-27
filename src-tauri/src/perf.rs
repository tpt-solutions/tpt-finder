// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Process memory probing for profiling / benchmarks (Phase 7).
//!
//! Working-set based; good enough to track index footprint over time.
//! Best-effort: returns 0 when the platform probe fails.

/// Current resident set size of this process, in bytes (0 if unknown).
pub fn rss_bytes() -> u64 {
    #[cfg(windows)]
    {
        windows_rss().unwrap_or(0)
    }
    #[cfg(not(windows))]
    {
        linux_rss().unwrap_or(0)
    }
}

/// Peak resident set size of this process, in bytes (0 if unknown).
pub fn peak_rss_bytes() -> u64 {
    #[cfg(windows)]
    {
        windows_peak_rss().unwrap_or(0)
    }
    #[cfg(not(windows))]
    {
        // /proc only exposes current RSS via statm; fall back to it.
        linux_rss().unwrap_or(0)
    }
}

#[cfg(windows)]
fn windows_counters() -> Option<(u64, u64)> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    let mut pmc = PROCESS_MEMORY_COUNTERS::default();
    let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // Pseudo-handle for the current process.
    let handle = HANDLE(-1isize as *mut _);
    let ok = unsafe { GetProcessMemoryInfo(handle, &mut pmc, cb) };
    if ok.is_ok() {
        Some((pmc.WorkingSetSize as u64, pmc.PeakWorkingSetSize as u64))
    } else {
        None
    }
}

#[cfg(windows)]
fn windows_rss() -> Option<u64> {
    windows_counters().map(|(rss, _)| rss)
}

#[cfg(windows)]
fn windows_peak_rss() -> Option<u64> {
    windows_counters().map(|(_, peak)| peak)
}

#[cfg(not(windows))]
fn linux_rss() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_is_reported_or_zero() {
        // Either the platform probe works (non-zero) or it degrades to 0 —
        // the contract is "never panic".
        let _ = rss_bytes();
        let _ = peak_rss_bytes();
    }
}
