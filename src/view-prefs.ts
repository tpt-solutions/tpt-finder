// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Result-list view preferences (Phase 8): visible columns, default sort,
// row density. Persisted per-machine in localStorage — window chrome local
// state, not something the backend needs to know about.

import type { SortKey } from "./shared";

export interface ViewPrefs {
	/** Columns shown besides Name (always visible). */
	path: boolean;
	size: boolean;
	modified: boolean;
	/** Default sort applied to fresh results. */
	sortKey: SortKey;
	sortDir: 1 | -1;
	/** Compact rows. */
	compact: boolean;
}

const DEFAULTS: ViewPrefs = {
	path: true,
	size: true,
	modified: true,
	sortKey: "name",
	sortDir: 1,
	compact: false,
};

const KEY = "tpt.view.prefs";

export function loadViewPrefs(): ViewPrefs {
	try {
		const raw = localStorage.getItem(KEY);
		if (!raw) return { ...DEFAULTS };
		const parsed = JSON.parse(raw) as Partial<ViewPrefs>;
		return {
			path: parsed.path ?? DEFAULTS.path,
			size: parsed.size ?? DEFAULTS.size,
			modified: parsed.modified ?? DEFAULTS.modified,
			sortKey: isSortKey(parsed.sortKey) ? parsed.sortKey : DEFAULTS.sortKey,
			sortDir: parsed.sortDir === -1 ? -1 : 1,
			compact: parsed.compact ?? DEFAULTS.compact,
		};
	} catch {
		return { ...DEFAULTS };
	}
}

export function saveViewPrefs(prefs: ViewPrefs): void {
	try {
		localStorage.setItem(KEY, JSON.stringify(prefs));
	} catch {
		// Storage unavailable (e.g. cleared webview data) — prefs just won't persist.
	}
}

function isSortKey(v: unknown): v is SortKey {
	return v === "name" || v === "size" || v === "modified" || v === "path";
}

/**
 * Apply prefs to the DOM: toggles visibility classes on the results list and
 * table head, marks the active sort button, and reflects checkboxes/select.
 */
export function applyViewPrefs(
	prefs: ViewPrefs,
	els: {
		results: HTMLElement;
		tableHead?: HTMLElement | null;
		sortButtons?: NodeListOf<HTMLButtonElement>;
	},
): void {
	const r = els.results;
	r.classList.toggle("no-path", !prefs.path);
	r.classList.toggle("no-size", !prefs.size);
	r.classList.toggle("no-modified", !prefs.modified);
	r.classList.toggle("compact", prefs.compact);
	if (els.tableHead) {
		els.tableHead.classList.toggle("no-path", !prefs.path);
		els.tableHead.classList.toggle("no-size", !prefs.size);
		els.tableHead.classList.toggle("no-modified", !prefs.modified);
	}
	for (const btn of els.sortButtons ?? []) {
		const key = btn.dataset.sort;
		const active = key === prefs.sortKey;
		btn.setAttribute("aria-pressed", active ? "true" : "false");
		let ind = btn.querySelector<HTMLElement>(".sort-indicator");
		if (!ind) {
			ind = document.createElement("span");
			ind.className = "sort-indicator";
			ind.setAttribute("aria-hidden", "true");
			btn.appendChild(ind);
		}
		ind.textContent = active ? (prefs.sortDir === 1 ? "▲" : "▼") : "";
	}
}
