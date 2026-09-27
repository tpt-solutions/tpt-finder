// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Popup overlay entry: type-as-you-type search, Esc to dismiss, Enter to open.

import { invoke } from "@tauri-apps/api/core";
import { ResultList } from "./results";
import {
	type IndexStatus,
	debounce,
	fetchSettings,
	fetchStatusReady,
	listenPopupShow,
	listenSettingsChanged,
	listenStatus,
	searchQuery,
} from "./shared";
import { applyTheme, watchSystemTheme } from "./theme";

document.body.classList.add("popup-mode");

const app = document.querySelector<HTMLDivElement>("#app");

if (app) {
	app.innerHTML = `
    <main class="popup-root">
      <input
        id="popup-input"
        type="search"
        placeholder="Search files…  ext:pdf  size:>10mb  path:docs"
        autocomplete="off"
        spellcheck="false"
        aria-label="Search files"
        autofocus
      />
      <p id="popup-status" class="status" aria-live="polite" role="status"></p>
      <ul id="popup-results" class="results" role="listbox" aria-label="Search results"></ul>
    </main>
  `;
}

// Theme follows the saved setting; updated live when settings change.
watchSystemTheme();
listenSettingsChanged((s) => applyTheme(s.theme));
void fetchSettings()
	.then((s) => applyTheme(s.theme))
	.catch(() => applyTheme("system"));

const input = document.querySelector<HTMLInputElement>("#popup-input");
const statusEl = document.querySelector<HTMLElement>("#popup-status");
const resultsEl = document.querySelector<HTMLUListElement>("#popup-results");

let list: ResultList | null = null;
if (resultsEl) {
	list = new ResultList({
		list: resultsEl,
		statusEl,
		afterOpen: () => {
			void invoke("hide_popup");
		},
	});
}

const runSearch = debounce((q: string) => {
	if (!list) return;
	void (async () => {
		try {
			const result = await searchQuery(q, 100);
			list?.setResult(result);
		} catch (e) {
			if (statusEl) statusEl.textContent = `Search failed: ${String(e)}`;
		}
	})();
}, 50);

input?.addEventListener("input", () => {
	runSearch(input.value);
});

document.addEventListener("keydown", (event) => {
	if (event.key === "Escape") {
		event.preventDefault();
		void invoke("hide_popup");
		return;
	}
	if (event.key === "Enter" && !(event.ctrlKey || event.metaKey)) {
		event.preventDefault();
		void list?.activateSelected();
		return;
	}
	if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
		// Reveal selected in folder.
		event.preventDefault();
		const item = list?.current();
		if (item) void invoke("reveal_path", { path: item.path });
		return;
	}
	list?.handleKey(event);
});

// Dismiss on blur of the whole popup document (no floating palette left open).
window.addEventListener("blur", () => {
	// Delay so click-through / internal focus transfers don't immediately hide.
	setTimeout(() => {
		if (document.hasFocus()) return;
		void invoke("hide_popup");
	}, 120);
});

function focusInput(): void {
	if (!input) return;
	input.focus();
	input.select();
}

listenPopupShow(() => {
	focusInput();
	if (input) runSearch(input.value);
});

listenStatus((s: IndexStatus) => {
	if (!statusEl) return;
	if (s.building) {
		statusEl.textContent = `Indexing… ${s.total_entries.toLocaleString()} entries`;
	}
});

void fetchStatusReady()
	.then((s) => {
		const st = s as IndexStatus;
		if (statusEl && !statusEl.textContent) {
			statusEl.textContent = st.building
				? `Indexing… ${st.total_entries.toLocaleString()}`
				: `${st.file_count.toLocaleString()} files · ready`;
		}
	})
	.catch(() => {
		if (statusEl) statusEl.textContent = "Backend unavailable.";
	});

focusInput();
