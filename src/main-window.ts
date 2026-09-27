// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Main window: full search UI, sortable table, preview pane, settings,
// view options, first-run onboarding.

import { invoke } from "@tauri-apps/api/core";
import { Preview } from "./preview";
import { ResultList } from "./results";
import { bindSettings } from "./settings-ui";
import {
	type IndexStatus,
	type SearchResultItem,
	type Settings,
	type SortKey,
	fetchSettings,
	fetchStatusReady,
	listenSettingsChanged,
	listenStatus,
	parseNlQuery,
	searchHybrid,
} from "./shared";
import { applyTheme, watchSystemTheme } from "./theme";
import { type ViewPrefs, applyViewPrefs, loadViewPrefs, saveViewPrefs } from "./view-prefs";

const app = document.querySelector<HTMLDivElement>("#app");

if (app) {
	app.innerHTML = `
    <main class="container">
      <header class="header">
        <h1>tpt-finder</h1>
        <span id="backend-badge" class="badge">…</span>
        <span class="header-spacer"></span>
        <button type="button" id="settings-toggle" class="secondary" aria-expanded="false" aria-controls="settings-panel">Settings</button>
      </header>

      <section id="onboarding" class="onboarding" hidden aria-labelledby="onboard-title">
        <h2 id="onboard-title">Welcome to tpt-finder</h2>
        <ol>
          <li>Initial index build — this walks every drive once. <span id="onboard-progress" class="onboard-progress"></span></li>
          <li>Press <kbd>Ctrl+Space</kbd> anywhere to summon the quick-search popup.</li>
          <li>Optional: connect a local <strong>Ollama</strong> instance for semantic search (“the PDF invoice from John”).</li>
        </ol>
        <div class="settings-actions">
          <button type="button" id="onboard-ollama" class="secondary">Set up Ollama…</button>
          <button type="button" id="onboard-dismiss">Got it</button>
        </div>
      </section>

      <form id="search-form" class="search-form" action="">
        <input
          id="search-input"
          name="q"
          type="search"
          placeholder="Search files…  ext:pdf  size:>10mb  path:docs  dated:2026-03"
          autocomplete="off"
          spellcheck="false"
          aria-label="Search files"
          autofocus
        />
        <button type="submit">Search</button>
        <button type="button" id="nl-btn" class="secondary" title="Parse natural language via local Ollama" aria-label="Parse natural language query">NL</button>
        <button type="button" id="rebuild-btn" class="secondary" title="Full rescan" aria-label="Rebuild index">Rebuild</button>
      </form>

      <div class="view-bar" role="group" aria-label="Result view options">
        <span class="view-group" aria-label="Visible columns">
          Columns:
          <label class="checkbox"><input type="checkbox" id="view-col-path" /> Path</label>
          <label class="checkbox"><input type="checkbox" id="view-col-size" /> Size</label>
          <label class="checkbox"><input type="checkbox" id="view-col-modified" /> Modified</label>
        </span>
        <span class="view-group">
          <label for="view-sort">Sort</label>
          <select id="view-sort">
            <option value="name">Name</option>
            <option value="path">Path</option>
            <option value="size">Size</option>
            <option value="modified">Modified</option>
          </select>
          <label for="view-sort-dir" class="sr-only">Sort direction</label>
          <select id="view-sort-dir">
            <option value="1">↑</option>
            <option value="-1">↓</option>
          </select>
        </span>
        <label class="checkbox"><input type="checkbox" id="view-compact" /> Compact rows</label>
      </div>

      <section id="settings-panel" class="settings-panel" hidden>
        <form id="settings-form" class="settings-form">
          <label>
            Global hotkey
            <input id="set-hotkey" name="hotkey" type="text" placeholder="CommandOrControl+Space" />
          </label>
          <label class="checkbox">
            <input id="set-show-main" name="show_main" type="checkbox" />
            Show main window on start
          </label>
          <label>
            Theme
            <select id="set-theme">
              <option value="system">System</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </label>
          <label>
            Search limit
            <input id="set-limit" name="limit" type="number" min="1" max="2000" />
          </label>

          <fieldset class="settings-group">
            <legend>Indexing exclusions (content extraction)</legend>
            <label>
              Excluded extensions (comma-separated, no dots)
              <textarea id="set-exclude-ext" rows="2" placeholder="exe, dll, png, mp4…"></textarea>
            </label>
            <label>
              Excluded folders (comma-separated)
              <textarea id="set-exclude-dirs" rows="2" placeholder="node_modules, .git, .cache…"></textarea>
            </label>
            <div class="settings-row">
              <label>
                Max extract size (MB)
                <input id="set-max-extract-mb" type="number" min="1" max="4096" />
              </label>
              <label>
                Extraction throttle (ms)
                <input id="set-extract-throttle" type="number" min="0" max="5000" />
              </label>
            </div>
            <label class="checkbox">
              <input id="set-error-log" type="checkbox" />
              Local error log (stored in app data, never uploaded)
            </label>
            <div class="settings-actions">
              <button type="button" id="set-clear-log" class="secondary">Clear error log</button>
              <span id="set-error-log-info" class="settings-hint" aria-live="polite"></span>
            </div>
          </fieldset>

          <fieldset class="settings-group">
            <legend>Semantic search (Ollama, local-only)</legend>
            <label class="checkbox">
              <input id="set-semantic-enabled" name="semantic_enabled" type="checkbox" />
              Enable semantic layer
            </label>
            <label>
              Ollama URL (localhost only)
              <input id="set-ollama-url" name="ollama_url" type="text" placeholder="http://127.0.0.1:11434" />
            </label>
            <label>
              Embedding model
              <input id="set-embed-model" name="embedding_model" type="text" placeholder="nomic-embed-text" />
            </label>
            <label>
              Chat model (NL query parsing)
              <input id="set-chat-model" name="chat_model" type="text" placeholder="llama3.2" />
            </label>
            <div class="settings-row">
              <label>
                Chunk chars
                <input id="set-chunk-chars" name="chunk_chars" type="number" min="64" max="8192" />
              </label>
              <label>
                Max chunks/file
                <input id="set-max-chunks" name="max_chunks" type="number" min="1" max="256" />
              </label>
              <label>
                Throttle (ms)
                <input id="set-embed-throttle" name="embed_throttle" type="number" min="0" max="5000" />
              </label>
            </div>
            <div class="settings-actions">
              <button type="button" id="set-ollama-check" class="secondary">Check Ollama</button>
              <button type="button" id="set-embed-rebuild" class="secondary" title="Background embedding pass">Embed content</button>
              <span id="set-ollama-health" class="settings-msg" aria-live="polite"></span>
            </div>
          </fieldset>

          <div class="settings-actions">
            <button type="submit">Save settings</button>
            <span id="settings-msg" class="settings-msg" aria-live="polite"></span>
          </div>
        </form>
      </section>

      <p id="status" class="status" aria-live="polite">Starting…</p>
      <div id="meta" class="meta"></div>

      <div class="workspace">
        <div class="results-pane">
          <div id="table-head" class="table-head">
            <button type="button" class="sort-btn" data-sort="name" aria-label="Sort by name">Name</button>
            <button type="button" class="sort-btn col-path" data-sort="path" aria-label="Sort by path">Path</button>
            <button type="button" class="sort-btn col-size" data-sort="size" aria-label="Sort by size">Size</button>
            <button type="button" class="sort-btn col-modified" data-sort="modified" aria-label="Sort by modified date">Modified</button>
          </div>
          <ul id="results" class="results" role="listbox" aria-label="Search results"></ul>
        </div>
        <aside id="preview" class="preview" aria-live="polite" aria-label="File preview">
          <p class="preview-empty">Select a result to preview.</p>
        </aside>
      </div>
    </main>
  `;
}

const statusEl = document.querySelector<HTMLParagraphElement>("#status");
const metaEl = document.querySelector<HTMLDivElement>("#meta");
const badgeEl = document.querySelector<HTMLSpanElement>("#backend-badge");
const form = document.querySelector<HTMLFormElement>("#search-form");
const input = document.querySelector<HTMLInputElement>("#search-input");
const resultsEl = document.querySelector<HTMLUListElement>("#results");
const tableHeadEl = document.querySelector<HTMLElement>("#table-head");
const rebuildBtn = document.querySelector<HTMLButtonElement>("#rebuild-btn");
const previewRoot = document.querySelector<HTMLElement>("#preview");
const settingsToggle = document.querySelector<HTMLButtonElement>("#settings-toggle");
const settingsPanel = document.querySelector<HTMLElement>("#settings-panel");

const preview = previewRoot ? new Preview(previewRoot) : null;

let list: ResultList | null = null;
if (resultsEl) {
	list = new ResultList({
		list: resultsEl,
		statusEl,
		showActions: true,
		sortable: true,
		onSelect: (item: SearchResultItem | null) => preview?.show(item),
	});
}

// ── View options (columns / sort / density) ─────────────────────────

const viewPrefs: ViewPrefs = loadViewPrefs();
const sortButtons = document.querySelectorAll<HTMLButtonElement>(".sort-btn");

function refreshViewUi(): void {
	if (!resultsEl) return;
	applyViewPrefs(viewPrefs, { results: resultsEl, tableHead: tableHeadEl, sortButtons });
	const colPath = document.querySelector<HTMLInputElement>("#view-col-path");
	const colSize = document.querySelector<HTMLInputElement>("#view-col-size");
	const colMod = document.querySelector<HTMLInputElement>("#view-col-modified");
	const sortSel = document.querySelector<HTMLSelectElement>("#view-sort");
	const dirSel = document.querySelector<HTMLSelectElement>("#view-sort-dir");
	const compact = document.querySelector<HTMLInputElement>("#view-compact");
	if (colPath) colPath.checked = viewPrefs.path;
	if (colSize) colSize.checked = viewPrefs.size;
	if (colMod) colMod.checked = viewPrefs.modified;
	if (sortSel) sortSel.value = viewPrefs.sortKey;
	if (dirSel) dirSel.value = String(viewPrefs.sortDir);
	if (compact) compact.checked = viewPrefs.compact;
}

refreshViewUi();
list?.setSortExact(viewPrefs.sortKey, viewPrefs.sortDir);

document.querySelector<HTMLInputElement>("#view-col-path")?.addEventListener("change", (ev) => {
	viewPrefs.path = (ev.target as HTMLInputElement).checked;
	saveViewPrefs(viewPrefs);
	refreshViewUi();
});
document.querySelector<HTMLInputElement>("#view-col-size")?.addEventListener("change", (ev) => {
	viewPrefs.size = (ev.target as HTMLInputElement).checked;
	saveViewPrefs(viewPrefs);
	refreshViewUi();
});
document.querySelector<HTMLInputElement>("#view-col-modified")?.addEventListener("change", (ev) => {
	viewPrefs.modified = (ev.target as HTMLInputElement).checked;
	saveViewPrefs(viewPrefs);
	refreshViewUi();
});
document.querySelector<HTMLSelectElement>("#view-sort")?.addEventListener("change", (ev) => {
	viewPrefs.sortKey = (ev.target as HTMLSelectElement).value as SortKey;
	saveViewPrefs(viewPrefs);
	list?.setSortExact(viewPrefs.sortKey, viewPrefs.sortDir);
	refreshViewUi();
});
document.querySelector<HTMLSelectElement>("#view-sort-dir")?.addEventListener("change", (ev) => {
	viewPrefs.sortDir = (ev.target as HTMLSelectElement).value === "-1" ? -1 : 1;
	saveViewPrefs(viewPrefs);
	list?.setSortExact(viewPrefs.sortKey, viewPrefs.sortDir);
	refreshViewUi();
});
document.querySelector<HTMLInputElement>("#view-compact")?.addEventListener("change", (ev) => {
	viewPrefs.compact = (ev.target as HTMLInputElement).checked;
	saveViewPrefs(viewPrefs);
	refreshViewUi();
});

// ── Status / search ─────────────────────────────────────────────────

let firstStatusSeen = false;

function renderStatus(s: IndexStatus): void {
	if (badgeEl) {
		badgeEl.textContent = s.building ? "indexing…" : s.backend;
		badgeEl.classList.toggle("busy", s.building);
	}
	if (statusEl && !list?.hasResults()) {
		// ResultList owns the status line once search results exist; before first search show index stats.
		if (s.building) {
			statusEl.textContent = `Building index… ${s.total_entries.toLocaleString()} entries so far`;
		} else if (s.total_entries === 0 && s.errors > 0) {
			statusEl.textContent = `Index build failed (${s.errors} errors) — try running as administrator.`;
		} else {
			statusEl.textContent = `${s.file_count.toLocaleString()} files · ${s.dir_count.toLocaleString()}${s.watch_active ? " · live" : ""}${s.errors ? ` · ${s.errors} errors` : ""}`;
		}
	}
	if (metaEl) {
		metaEl.textContent = s.volumes
			.map((v) => `${v.root} (${v.fs}${v.live_updates ? "" : ", scan-only"})`)
			.join("  ·  ");
	}
	updateOnboarding(s);
	firstStatusSeen = true;
}

async function refreshStatus(): Promise<void> {
	try {
		const st = await fetchStatusReady();
		renderStatus(st);
	} catch (e) {
		if (statusEl) statusEl.textContent = `Status error: ${String(e)}`;
	}
}

const runSearch = debounce((q: string) => {
	void (async () => {
		if (!list) return;
		try {
			// Hybrid degrades to plain filename search when semantic is off.
			const result = await searchHybrid(q, 200);
			list.setResult(result);
		} catch (e) {
			if (resultsEl) {
				resultsEl.innerHTML = `<li class="empty">Search failed: ${String(e)}</li>`;
			}
		}
	})();
}, 80);

function debounce(fn: (q: string) => void, ms: number): (q: string) => void {
	let t: ReturnType<typeof setTimeout> | undefined;
	return (q: string) => {
		if (t) clearTimeout(t);
		t = setTimeout(() => fn(q), ms);
	};
}

form?.addEventListener("submit", (event) => {
	event.preventDefault();
	if (input) runSearch(input.value);
});

input?.addEventListener("input", () => {
	if (input) runSearch(input.value);
});

// Explicit natural-language parse (local Ollama chat model) → structured query.
const nlBtn = document.querySelector<HTMLButtonElement>("#nl-btn");
nlBtn?.addEventListener("click", () => {
	void (async () => {
		if (!input || !list) return;
		const raw = input.value.trim();
		if (!raw) return;
		nlBtn.disabled = true;
		const prev = nlBtn.textContent;
		nlBtn.textContent = "…";
		try {
			const parsed = await parseNlQuery(raw);
			const structured = buildQuery(parsed);
			if (structured && structured !== raw) {
				input.value = structured;
			}
			const result = await searchHybrid(input.value, 200);
			list.setResult(result);
		} catch (e) {
			if (resultsEl) {
				resultsEl.innerHTML = `<li class="empty">NL parse failed: ${String(e)}</li>`;
			}
		} finally {
			nlBtn.disabled = false;
			nlBtn.textContent = prev;
		}
	})();
});

/** Recompose an Everything-style query from the structured NL parse. */
function buildQuery(p: {
	terms: string;
	ext: string[];
	path: string[];
	dated: string | null;
}): string {
	const parts: string[] = [];
	if (p.terms.trim()) parts.push(p.terms.trim());
	for (const e of p.ext) {
		const clean = e.replace(/^\.+/, "").toLowerCase();
		if (clean) parts.push(`ext:${clean}`);
	}
	for (const seg of p.path) {
		const t = seg.trim();
		if (t) parts.push(`path:${t.replace(/ /g, "_")}`);
	}
	if (p.dated?.trim()) parts.push(`dated:${p.dated.trim()}`);
	return parts.join(" ");
}

rebuildBtn?.addEventListener("click", () => {
	void invoke("rebuild_index").then(refreshStatus).catch(console.error);
});

settingsToggle?.addEventListener("click", () => {
	if (!settingsPanel || !settingsToggle) return;
	settingsPanel.hidden = !settingsPanel.hidden;
	settingsToggle.setAttribute("aria-expanded", settingsPanel.hidden ? "false" : "true");
});

// Sortable column headers.
for (const btn of sortButtons) {
	btn.addEventListener("click", () => {
		const key = btn.dataset.sort as SortKey | undefined;
		if (!key || !list) return;
		if (viewPrefs.sortKey === key) {
			viewPrefs.sortDir = viewPrefs.sortDir === 1 ? -1 : 1;
			list.setSortExact(key, viewPrefs.sortDir);
		} else {
			viewPrefs.sortKey = key;
			viewPrefs.sortDir = key === "size" || key === "modified" ? -1 : 1;
			list.setSortExact(key, viewPrefs.sortDir);
		}
		saveViewPrefs(viewPrefs);
		refreshViewUi();
	});
}

document.addEventListener("keydown", (event) => {
	// Don't steal keys while typing in the search box for Enter (form handles it).
	if (event.key === "Escape" && document.activeElement === input) {
		if (input) input.value = "";
		runSearch("");
		return;
	}
	if (event.key === "/" && document.activeElement !== input && !(event.ctrlKey || event.metaKey)) {
		event.preventDefault();
		input?.focus();
		return;
	}
	list?.handleKey(event);
	if (event.key === "Enter" && document.activeElement !== input) {
		void list?.activateSelected();
	}
});

// ── Settings panel wiring ───────────────────────────────────────────

const settingsForm = document.querySelector<HTMLFormElement>("#settings-form");
const setHotkey = document.querySelector<HTMLInputElement>("#set-hotkey");
const setShowMain = document.querySelector<HTMLInputElement>("#set-show-main");
const setLimit = document.querySelector<HTMLInputElement>("#set-limit");
const setMsg = document.querySelector<HTMLElement>("#settings-msg");

const semanticOk =
	document.querySelector<HTMLInputElement>("#set-semantic-enabled") &&
	document.querySelector<HTMLInputElement>("#set-ollama-url") &&
	document.querySelector<HTMLInputElement>("#set-embed-model") &&
	document.querySelector<HTMLInputElement>("#set-chat-model") &&
	document.querySelector<HTMLInputElement>("#set-chunk-chars") &&
	document.querySelector<HTMLInputElement>("#set-max-chunks") &&
	document.querySelector<HTMLInputElement>("#set-embed-throttle") &&
	document.querySelector<HTMLElement>("#set-ollama-health") &&
	document.querySelector<HTMLButtonElement>("#set-ollama-check") &&
	document.querySelector<HTMLButtonElement>("#set-embed-rebuild");

if (settingsForm && setHotkey && setShowMain && setLimit && setMsg) {
	bindSettings({
		form: settingsForm,
		hotkey: setHotkey,
		showMain: setShowMain,
		limit: setLimit,
		msg: setMsg,
		theme: document.querySelector<HTMLSelectElement>("#set-theme") ?? undefined,
		excludeExtensions: document.querySelector<HTMLTextAreaElement>("#set-exclude-ext") ?? undefined,
		excludeDirs: document.querySelector<HTMLTextAreaElement>("#set-exclude-dirs") ?? undefined,
		maxExtractMb: document.querySelector<HTMLInputElement>("#set-max-extract-mb") ?? undefined,
		extractThrottle: document.querySelector<HTMLInputElement>("#set-extract-throttle") ?? undefined,
		errorLog: document.querySelector<HTMLInputElement>("#set-error-log") ?? undefined,
		errorLogInfo: document.querySelector<HTMLElement>("#set-error-log-info") ?? undefined,
		clearLog: document.querySelector<HTMLButtonElement>("#set-clear-log") ?? undefined,
		semantic: semanticOk
			? {
					enabled: document.querySelector<HTMLInputElement>(
						"#set-semantic-enabled",
					) as HTMLInputElement,
					url: document.querySelector<HTMLInputElement>("#set-ollama-url") as HTMLInputElement,
					embeddingModel: document.querySelector<HTMLInputElement>(
						"#set-embed-model",
					) as HTMLInputElement,
					chatModel: document.querySelector<HTMLInputElement>(
						"#set-chat-model",
					) as HTMLInputElement,
					chunkChars: document.querySelector<HTMLInputElement>(
						"#set-chunk-chars",
					) as HTMLInputElement,
					maxChunks: document.querySelector<HTMLInputElement>(
						"#set-max-chunks",
					) as HTMLInputElement,
					throttleMs: document.querySelector<HTMLInputElement>(
						"#set-embed-throttle",
					) as HTMLInputElement,
					health: document.querySelector<HTMLElement>("#set-ollama-health") as HTMLElement,
					check: document.querySelector<HTMLButtonElement>(
						"#set-ollama-check",
					) as HTMLButtonElement,
					rebuild: document.querySelector<HTMLButtonElement>(
						"#set-embed-rebuild",
					) as HTMLButtonElement,
				}
			: undefined,
	});
}

// ── First-run onboarding ────────────────────────────────────────────

const ONBOARD_KEY = "tpt.onboarded";
const onboardingEl = document.querySelector<HTMLElement>("#onboarding");
const onboardProgress = document.querySelector<HTMLElement>("#onboard-progress");
let onboardingActive = false;

function maybeShowOnboarding(s: Settings | null): void {
	if (!onboardingEl || localStorage.getItem(ONBOARD_KEY)) return;
	// Show only when the index is still cold (first run heuristic); a settings
	// fetch failure shouldn't block it, so s === null still shows the panel.
	if (s && !s.show_main_on_start) {
		// Headless-ish usage: skip the tour.
		return;
	}
	onboardingEl.hidden = false;
	onboardingActive = true;
}

function updateOnboarding(s: IndexStatus): void {
	if (!onboardingActive || !onboardProgress) return;
	if (s.building) {
		onboardProgress.textContent = `${s.total_entries.toLocaleString()} entries indexed…`;
	} else if (s.total_entries > 0) {
		onboardProgress.textContent = `done — ${s.file_count.toLocaleString()} files ready.`;
	}
}

document.querySelector<HTMLButtonElement>("#onboard-dismiss")?.addEventListener("click", () => {
	if (onboardingEl) onboardingEl.hidden = true;
	onboardingActive = false;
	localStorage.setItem(ONBOARD_KEY, "1");
});

document.querySelector<HTMLButtonElement>("#onboard-ollama")?.addEventListener("click", () => {
	if (onboardingEl) onboardingEl.hidden = true;
	onboardingActive = false;
	localStorage.setItem(ONBOARD_KEY, "1");
	if (settingsPanel && settingsToggle) {
		settingsPanel.hidden = false;
		settingsToggle.setAttribute("aria-expanded", "true");
		settingsPanel.scrollIntoView({ block: "start" });
		document.querySelector<HTMLInputElement>("#set-semantic-enabled")?.focus();
	}
});

// ── Theme ───────────────────────────────────────────────────────────

watchSystemTheme();
listenSettingsChanged((s) => applyTheme(s.theme));
void fetchSettings()
	.then((s) => {
		applyTheme(s.theme);
		maybeShowOnboarding(s);
	})
	.catch(() => {
		applyTheme("system");
		if (!firstStatusSeen) maybeShowOnboarding(null);
	});

// ── Init ────────────────────────────────────────────────────────────

async function init(): Promise<void> {
	listenStatus(renderStatus);
	try {
		const msg = await invoke<string>("ping");
		if (statusEl && !statusEl.textContent?.includes("files")) {
			statusEl.textContent = msg;
		}
	} catch {
		if (statusEl) statusEl.textContent = "Backend unavailable.";
	}
	await refreshStatus();
	input?.focus();
}

void init();
