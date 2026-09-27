// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Settings panel (main window): hotkey, launch behavior, theme, search limit,
// exclusion rules, error log, semantic layer.

import {
	type ErrorLogInfo,
	type OllamaHealth,
	type Settings,
	fetchErrorLogInfo,
	fetchOllamaHealth,
	fetchSettings,
	saveSettings,
} from "./shared";
import { applyTheme } from "./theme";

export interface SemanticEls {
	enabled: HTMLInputElement;
	url: HTMLInputElement;
	embeddingModel: HTMLInputElement;
	chatModel: HTMLInputElement;
	chunkChars: HTMLInputElement;
	maxChunks: HTMLInputElement;
	throttleMs: HTMLInputElement;
	health: HTMLElement;
	check: HTMLButtonElement;
	rebuild: HTMLButtonElement;
}

export interface SettingsPanelEls {
	form: HTMLFormElement;
	hotkey: HTMLInputElement;
	showMain: HTMLInputElement;
	limit: HTMLInputElement;
	msg: HTMLElement;
	theme?: HTMLSelectElement;
	excludeExtensions?: HTMLTextAreaElement;
	excludeDirs?: HTMLTextAreaElement;
	maxExtractMb?: HTMLInputElement;
	extractThrottle?: HTMLInputElement;
	errorLog?: HTMLInputElement;
	errorLogInfo?: HTMLElement;
	clearLog?: HTMLButtonElement;
	semantic?: SemanticEls;
}

function clamp(n: number, lo: number, hi: number): number {
	return Math.max(lo, Math.min(hi, n));
}

function parseList(text: string): string[] {
	return text
		.split(/[\n,;]+/)
		.map((s) => s.trim())
		.filter(Boolean);
}

export function bindSettings(els: SettingsPanelEls): void {
	void loadInto(els);
	if (els.semantic) bindSemantic(els.semantic);
	if (els.clearLog && els.errorLogInfo) {
		els.clearLog.addEventListener("click", () => {
			void (async () => {
				try {
					const { clearErrorLog } = await import("./shared");
					await clearErrorLog();
					await refreshErrorLogInfo(els);
				} catch (e) {
					if (els.errorLogInfo) els.errorLogInfo.textContent = String(e);
				}
			})();
		});
	}

	els.form.addEventListener("submit", (ev) => {
		ev.preventDefault();
		void saveFrom(els);
	});
}

async function refreshErrorLogInfo(els: SettingsPanelEls): Promise<void> {
	if (!els.errorLogInfo) return;
	try {
		const info: ErrorLogInfo = await fetchErrorLogInfo();
		const kb = info.size_bytes > 0 ? ` · ${(info.size_bytes / 1024).toFixed(1)} KB` : "";
		els.errorLogInfo.textContent = `Local file, never uploaded${kb}`;
		els.errorLogInfo.title = info.path;
	} catch {
		els.errorLogInfo.textContent = "";
	}
}

async function loadInto(els: SettingsPanelEls): Promise<void> {
	try {
		const s = await fetchSettings();
		els.hotkey.value = s.hotkey;
		els.showMain.checked = s.show_main_on_start;
		els.limit.value = String(s.search_limit);
		if (els.theme) els.theme.value = s.theme === "light" || s.theme === "dark" ? s.theme : "system";
		if (els.excludeExtensions) els.excludeExtensions.value = s.exclude_extensions.join(", ");
		if (els.excludeDirs) els.excludeDirs.value = s.exclude_dirs.join(", ");
		if (els.maxExtractMb)
			els.maxExtractMb.value = String(Math.round(s.max_extract_bytes / (1024 * 1024)));
		if (els.extractThrottle) els.extractThrottle.value = String(s.extraction_throttle_ms);
		if (els.errorLog) els.errorLog.checked = s.error_log_enabled;
		if (els.semantic) {
			els.semantic.enabled.checked = s.semantic_enabled;
			els.semantic.url.value = s.ollama_url;
			els.semantic.embeddingModel.value = s.embedding_model;
			els.semantic.chatModel.value = s.chat_model;
			els.semantic.chunkChars.value = String(s.chunk_chars);
			els.semantic.maxChunks.value = String(s.max_chunks_per_file);
			els.semantic.throttleMs.value = String(s.embed_throttle_ms);
		}
		els.msg.textContent = "";
		void refreshErrorLogInfo(els);
	} catch (e) {
		els.msg.textContent = `Load failed: ${String(e)}`;
	}
}

async function saveFrom(els: SettingsPanelEls): Promise<void> {
	const sem = els.semantic;
	const settings: Settings = {
		hotkey: els.hotkey.value.trim() || "CommandOrControl+Space",
		show_main_on_start: els.showMain.checked,
		search_limit: clamp(Number(els.limit.value) || 200, 1, 2000),
		popup_center: true,
		theme: els.theme ? els.theme.value : "system",
		exclude_extensions: els.excludeExtensions ? parseList(els.excludeExtensions.value) : [],
		exclude_dirs: els.excludeDirs ? parseList(els.excludeDirs.value) : [],
		max_extract_bytes: els.maxExtractMb
			? clamp(Number(els.maxExtractMb.value) || 16, 1, 4096) * 1024 * 1024
			: 16 * 1024 * 1024,
		extraction_throttle_ms: els.extractThrottle
			? clamp(Number(els.extractThrottle.value) || 0, 0, 5000)
			: 0,
		error_log_enabled: els.errorLog ? els.errorLog.checked : false,
		semantic_enabled: sem ? sem.enabled.checked : false,
		ollama_url: sem ? sem.url.value.trim() || "http://127.0.0.1:11434" : "http://127.0.0.1:11434",
		embedding_model: sem
			? sem.embeddingModel.value.trim() || "nomic-embed-text"
			: "nomic-embed-text",
		chat_model: sem ? sem.chatModel.value.trim() || "llama3.2" : "llama3.2",
		chunk_chars: sem ? clamp(Number(sem.chunkChars.value) || 512, 64, 8192) : 512,
		max_chunks_per_file: sem ? clamp(Number(sem.maxChunks.value) || 32, 1, 256) : 32,
		embed_throttle_ms: sem ? clamp(Number(sem.throttleMs.value) || 50, 0, 5000) : 50,
	};
	try {
		const saved = await saveSettings(settings);
		// Reflect immediately in this window (the event covers the others).
		applyTheme(saved.theme);
		els.msg.textContent = "Saved.";
		if (sem) void checkHealth(sem, false);
		setTimeout(() => {
			if (els.msg.textContent === "Saved.") els.msg.textContent = "";
		}, 2000);
	} catch (e) {
		els.msg.textContent = String(e);
	}
}

function bindSemantic(els: SemanticEls): void {
	els.check.addEventListener("click", () => void checkHealth(els, true));
	els.rebuild.addEventListener("click", () => {
		void (async () => {
			els.rebuild.disabled = true;
			els.health.textContent = "Queued embedding pass…";
			try {
				const { rebuildSemantic } = await import("./shared");
				await rebuildSemantic(false);
				els.health.textContent = "Embedding pass started in background.";
			} catch (e) {
				els.health.textContent = `Rebuild failed: ${String(e)}`;
			} finally {
				els.rebuild.disabled = false;
			}
		})();
	});
	void checkHealth(els, false);
}

async function checkHealth(els: SemanticEls, verbose: boolean): Promise<void> {
	els.health.textContent = "Checking Ollama…";
	try {
		const h: OllamaHealth = await fetchOllamaHealth();
		if (!h.reachable) {
			els.health.textContent = `Ollama unreachable${h.error ? `: ${h.error}` : ""}`;
			return;
		}
		const missing: string[] = [];
		if (!h.embedding_model_available) missing.push(`embedding (${els.embeddingModel.value})`);
		if (!h.chat_model_available) missing.push(`chat (${els.chatModel.value})`);
		els.health.textContent = missing.length
			? `Ollama OK — missing ${missing.join(", ")}`
			: `Ollama OK — ${h.models.length} model(s)`;
		if (verbose && !missing.length) els.health.textContent += " ✓";
	} catch (e) {
		els.health.textContent = `Health check failed: ${String(e)}`;
	}
}
