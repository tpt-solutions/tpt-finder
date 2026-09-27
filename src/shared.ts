// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Shared types + helpers for main window and popup search UIs.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface SearchResultItem {
	id: number;
	path: string;
	name: string;
	size: number;
	is_dir: boolean;
	modified_ms: number;
	attributes: number;
	score: number;
}

export interface SearchResult {
	items: SearchResultItem[];
	total_matched: number;
	took_ms: number;
	query: string;
}

export interface VolumeStatus {
	root: string;
	fs: string;
	live_updates: boolean;
	indexed: boolean;
}

export interface IndexStatus {
	building: boolean;
	file_count: number;
	dir_count: number;
	total_entries: number;
	volumes: VolumeStatus[];
	backend: string;
	last_full_scan_ms: number;
	last_saved_ms: number;
	errors: number;
	watch_active: boolean;
	content_queued: number;
	content_extracted: number;
	content_failed: number;
}

export interface Settings {
	hotkey: string;
	show_main_on_start: boolean;
	search_limit: number;
	popup_center: boolean;
	semantic_enabled: boolean;
	ollama_url: string;
	embedding_model: string;
	chat_model: string;
	chunk_chars: number;
	max_chunks_per_file: number;
	embed_throttle_ms: number;
	theme: string;
	exclude_extensions: string[];
	exclude_dirs: string[];
	max_extract_bytes: number;
	extraction_throttle_ms: number;
	error_log_enabled: boolean;
}

export interface ErrorLogInfo {
	enabled: boolean;
	path: string;
	size_bytes: number;
}

export interface OllamaHealth {
	reachable: boolean;
	models: string[];
	embedding_model_available: boolean;
	chat_model_available: boolean;
	error: string | null;
}

export interface EmbedStats {
	queued: number;
	processed: number;
	succeeded: number;
	failed: number;
	skipped: number;
	running: boolean;
	last_error: string | null;
}

export interface ParsedNL {
	terms: string;
	ext: string[];
	path: string[];
	dated: string | null;
	semantic: boolean;
}

export interface VectorHit {
	path: string;
	chunk_index: number;
	score: number;
	snippet: string;
}

export type SortKey = "name" | "size" | "modified" | "path";

export function isPopupWindow(): boolean {
	const params = new URLSearchParams(window.location.search);
	return params.get("window") === "popup";
}

export function formatSize(n: number): string {
	if (n <= 0) return "—";
	const units = ["B", "KB", "MB", "GB", "TB"];
	let v = n;
	let i = 0;
	while (v >= 1024 && i < units.length - 1) {
		v /= 1024;
		i++;
	}
	return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export function formatModified(ms: number): string {
	if (ms <= 0) return "—";
	return new Date(ms).toLocaleDateString();
}

export function itemMeta(item: SearchResultItem): string {
	const bits: string[] = [];
	if (item.is_dir) bits.push("dir");
	else bits.push(formatSize(item.size));
	if (item.modified_ms > 0) bits.push(formatModified(item.modified_ms));
	return bits.join(" · ");
}

export function extOf(name: string): string {
	const i = name.lastIndexOf(".");
	return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

/** Kind label used for the preview pane / icon placeholder. */
export function kindOf(item: SearchResultItem): string {
	if (item.is_dir) return "Folder";
	const ext = extOf(item.name);
	return ext ? `${ext.toUpperCase()} file` : "File";
}

export async function searchQuery(query: string, limit?: number): Promise<SearchResult> {
	return invoke<SearchResult>("search", { query, limit });
}

export async function fetchStatus(): Promise<IndexStatus> {
	return invoke<IndexStatus>("index_status");
}

/**
 * fetchStatus with retries: on launch the webview can boot before the Rust
 * setup finishes (index restore takes seconds), and commands fail with
 * "state not managed" until `manage()` runs. Retry those with backoff.
 */
export async function fetchStatusReady(retries = 40, delayMs = 250): Promise<IndexStatus> {
	let lastError: unknown = null;
	for (let attempt = 0; attempt < retries; attempt++) {
		try {
			return await fetchStatus();
		} catch (e) {
			lastError = e;
			if (!String(e).toLowerCase().includes("not managed")) throw e;
			await new Promise((r) => setTimeout(r, delayMs));
		}
	}
	throw lastError;
}

export async function fetchSettings(): Promise<Settings> {
	return invoke<Settings>("get_settings");
}

export async function saveSettings(settings: Settings): Promise<Settings> {
	return invoke<Settings>("set_settings", { settings });
}

export async function fetchErrorLogInfo(): Promise<ErrorLogInfo> {
	return invoke<ErrorLogInfo>("error_log_info");
}

export async function clearErrorLog(): Promise<void> {
	return invoke("clear_error_log");
}

/** Fired by the backend after settings are saved (any window can react). */
export function listenSettingsChanged(cb: (s: Settings) => void): () => void {
	let unsub: (() => void) | undefined;
	void listen<Settings>("settings://changed", (ev) => cb(ev.payload)).then((f) => {
		unsub = f;
	});
	return () => unsub?.();
}

export async function fetchOllamaHealth(): Promise<OllamaHealth> {
	return invoke<OllamaHealth>("ollama_health");
}

export async function fetchSemanticStatus(): Promise<EmbedStats> {
	return invoke<EmbedStats>("semantic_status");
}

export async function fetchVectorCount(): Promise<number> {
	return invoke<number>("vector_count");
}

export async function rebuildSemantic(force: boolean): Promise<void> {
	return invoke("rebuild_semantic", { force });
}

export async function searchSemantic(query: string, limit?: number): Promise<VectorHit[]> {
	return invoke<VectorHit[]>("search_semantic", { query, limit });
}

export async function parseNlQuery(query: string): Promise<ParsedNL> {
	return invoke<ParsedNL>("parse_nl_query", { query });
}

/** Hybrid search: filename + full-text + vector merged, degrades to plain search. */
export async function searchHybrid(query: string, limit?: number): Promise<SearchResult> {
	return invoke<SearchResult>("search_hybrid", { query, limit });
}

export async function openItem(path: string): Promise<void> {
	return invoke("open_path", { path });
}

export async function revealItem(path: string): Promise<void> {
	return invoke("reveal_path", { path });
}

export async function deleteItem(path: string): Promise<void> {
	return invoke("delete_path", { path });
}

export async function copyItemPath(path: string): Promise<void> {
	return invoke("copy_path", { path });
}

export async function copyItemFile(path: string): Promise<void> {
	return invoke("copy_file", { path });
}

export function listenStatus(cb: (s: IndexStatus) => void): () => void {
	let unsub: (() => void) | undefined;
	void listen<IndexStatus>("index://status", (ev) => cb(ev.payload)).then((f) => {
		unsub = f;
	});
	return () => unsub?.();
}

export function listenPopupShow(cb: () => void): () => void {
	let unsub: (() => void) | undefined;
	void listen("popup://show", () => cb()).then((f) => {
		unsub = f;
	});
	return () => unsub?.();
}

export function debounce(fn: (q: string) => void, ms: number): (q: string) => void {
	let t: ReturnType<typeof setTimeout> | undefined;
	return (q: string) => {
		if (t) clearTimeout(t);
		t = setTimeout(() => fn(q), ms);
	};
}

/** Sort a snapshot of items without mutating the original array. */
export function sortItems(
	items: SearchResultItem[],
	key: SortKey,
	dir: 1 | -1,
): SearchResultItem[] {
	const copy = [...items];
	copy.sort((a, b) => {
		let cmp = 0;
		switch (key) {
			case "name":
				cmp = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
				break;
			case "size":
				cmp = Number(a.size) - Number(b.size);
				break;
			case "modified":
				cmp = a.modified_ms - b.modified_ms;
				break;
			case "path":
				cmp = a.path.localeCompare(b.path, undefined, { sensitivity: "base" });
				break;
		}
		return cmp * dir;
	});
	return copy;
}
