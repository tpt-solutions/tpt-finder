// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Light/dark/system theme application (Phase 8). Both windows share this.

export type ThemeChoice = "system" | "light" | "dark";

export function normalizeTheme(value: string | undefined | null): ThemeChoice {
	return value === "light" || value === "dark" ? value : "system";
}

/** Apply a theme choice to the document root. */
export function applyTheme(theme: string | undefined | null): ThemeChoice {
	const choice = normalizeTheme(theme);
	const root = document.documentElement;
	if (choice === "system") {
		delete root.dataset.theme;
	} else {
		root.dataset.theme = choice;
	}
	syncColorScheme();
	return choice;
}

/** Keep the native form-control scheme in sync (esp. "system" mode). */
export function syncColorScheme(): void {
	const root = document.documentElement;
	const forced = root.dataset.theme;
	if (forced === "light" || forced === "dark") {
		root.style.colorScheme = forced;
		return;
	}
	root.style.colorScheme = window.matchMedia("(prefers-color-scheme: dark)").matches
		? "dark"
		: "light";
}

/** Re-sync when the OS scheme flips while in "system" mode. */
export function watchSystemTheme(): void {
	window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
		syncColorScheme();
	});
}
