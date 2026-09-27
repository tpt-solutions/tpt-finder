// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Shared result-list + selection controller used by popup and main windows.

import {
	type SearchResult,
	type SearchResultItem,
	type SortKey,
	copyItemFile,
	copyItemPath,
	deleteItem,
	formatModified,
	formatSize,
	openItem,
	revealItem,
	sortItems,
} from "./shared";

export interface ResultListOptions {
	/** Container <ul> element. */
	list: HTMLUListElement;
	/** Optional status line updated with match counts. */
	statusEl?: HTMLElement | null;
	/** Max items to keep selected-index within. */
	onSelect?: (item: SearchResultItem | null) => void;
	/** Called when user activates (Enter / double-click). */
	onActivate?: (item: SearchResultItem) => void;
	/** After a successful open, notify (e.g. hide popup). */
	afterOpen?: () => void;
	/** Enable inline action buttons (main window). */
	showActions?: boolean;
	/** Sort mode for main table. */
	sortable?: boolean;
}

export class ResultList {
	private list: HTMLUListElement;
	private statusEl: HTMLElement | null;
	private onSelect: ((item: SearchResultItem | null) => void) | undefined;
	private onActivate: ((item: SearchResultItem) => void) | undefined;
	private afterOpen: (() => void) | undefined;
	private showActions: boolean;
	private sortable: boolean;

	private items: SearchResultItem[] = [];
	private selected = -1;
	private sortKey: SortKey = "name";
	private sortDir: 1 | -1 = 1;
	private lastTotal = 0;
	private lastTook = 0;
	private rawItems: SearchResultItem[] = [];

	constructor(opts: ResultListOptions) {
		this.list = opts.list;
		this.statusEl = opts.statusEl ?? null;
		this.onSelect = opts.onSelect;
		this.onActivate = opts.onActivate;
		this.afterOpen = opts.afterOpen;
		this.showActions = opts.showActions ?? false;
		this.sortable = opts.sortable ?? false;
	}

	setResult(result: SearchResult): void {
		this.rawItems = result.items;
		this.lastTotal = result.total_matched;
		this.lastTook = result.took_ms;
		this.applySortAndRender();
		this.selected = this.items.length > 0 ? 0 : -1;
		this.highlight();
		this.emitStatus();
		this.onSelect?.(this.current() ?? null);
	}

	current(): SearchResultItem | null {
		if (this.selected < 0 || this.selected >= this.items.length) return null;
		return this.items[this.selected];
	}

	/** Keyboard navigation. Returns true if the event was handled. */
	handleKey(event: KeyboardEvent): boolean {
		if (this.items.length === 0) return false;
		if (event.key === "ArrowDown") {
			event.preventDefault();
			this.select(Math.min(this.selected + 1, this.items.length - 1));
			return true;
		}
		if (event.key === "ArrowUp") {
			event.preventDefault();
			this.select(Math.max(this.selected - 1, 0));
			return true;
		}
		if (event.key === "Home") {
			event.preventDefault();
			this.select(0);
			return true;
		}
		if (event.key === "End") {
			event.preventDefault();
			this.select(this.items.length - 1);
			return true;
		}
		return false;
	}

	async activateSelected(): Promise<void> {
		const item = this.current();
		if (!item) return;
		if (this.onActivate) {
			this.onActivate(item);
			return;
		}
		await this.open(item);
	}

	async open(item: SearchResultItem): Promise<void> {
		try {
			await openItem(item.path);
			this.afterOpen?.();
		} catch (e) {
			this.flashStatus(`Open failed: ${String(e)}`);
		}
	}

	setSort(key: SortKey): void {
		if (this.sortKey === key) this.sortDir = this.sortDir === 1 ? -1 : 1;
		else {
			this.sortKey = key;
			this.sortDir = key === "size" || key === "modified" ? -1 : 1;
		}
		this.applySortAndRender();
		this.highlight();
	}

	/** Set sort key + direction explicitly (view options / persisted prefs). */
	setSortExact(key: SortKey, dir: 1 | -1): void {
		this.sortKey = key;
		this.sortDir = dir;
		this.applySortAndRender();
		this.highlight();
	}

	hasResults(): boolean {
		return this.rawItems.length > 0;
	}

	private applySortAndRender(): void {
		this.items = this.sortable
			? sortItems(this.rawItems, this.sortKey, this.sortDir)
			: this.rawItems;
		this.render();
	}

	private select(idx: number): void {
		this.selected = idx;
		this.highlight();
		this.onSelect?.(this.current() ?? null);
	}

	private highlight(): void {
		const nodes = this.list.querySelectorAll<HTMLLIElement>("li.result");
		nodes.forEach((el, i) => {
			el.classList.toggle("selected", i === this.selected);
			el.setAttribute("aria-selected", i === this.selected ? "true" : "false");
		});
		const active = nodes[this.selected];
		// Expose the focused option to assistive tech (listbox pattern).
		if (active?.id) {
			this.list.setAttribute("aria-activedescendant", active.id);
		} else {
			this.list.removeAttribute("aria-activedescendant");
		}
		active?.scrollIntoView({ block: "nearest" });
	}

	private emitStatus(): void {
		if (!this.statusEl) return;
		if (this.lastTotal === 0) {
			this.statusEl.textContent = "No matches.";
			return;
		}
		const shown = Math.min(this.items.length, this.lastTotal);
		const matched =
			this.lastTotal > shown
				? `${shown} of ${this.lastTotal.toLocaleString()} matches`
				: `${this.lastTotal.toLocaleString()} matches`;
		this.statusEl.textContent = `${matched} · ${this.lastTook} ms`;
	}

	private flashStatus(msg: string): void {
		if (!this.statusEl) return;
		this.statusEl.textContent = msg;
		setTimeout(() => {
			if (this.statusEl?.textContent === msg) this.emitStatus();
		}, 2500);
	}

	private render(): void {
		this.list.innerHTML = "";
		if (this.rawItems.length === 0) {
			const li = document.createElement("li");
			li.className = "empty";
			li.textContent = "No matches.";
			this.list.appendChild(li);
			return;
		}
		for (let i = 0; i < this.items.length; i++) {
			const item = this.items[i];
			const li = document.createElement("li");
			li.className = "result";
			li.setAttribute("role", "option");
			li.id = `${this.list.id || "results"}-opt-${i}`;
			li.tabIndex = -1;
			li.dataset.path = item.path;

			const name = document.createElement("span");
			name.className = "result-name";
			name.textContent = item.name;

			const path = document.createElement("span");
			path.className = "result-path";
			path.textContent = item.path;
			path.title = item.path;

			// Meta split into togglable column spans (view options).
			const meta = document.createElement("span");
			meta.className = "result-meta";
			const size = document.createElement("span");
			size.className = "result-size";
			size.textContent = item.is_dir ? "dir" : formatSize(item.size);
			const modified = document.createElement("span");
			modified.className = "result-modified";
			modified.textContent = item.modified_ms > 0 ? formatModified(item.modified_ms) : "—";
			meta.append(size, document.createTextNode(" · "), modified);

			li.append(name, path, meta);

			li.addEventListener("click", (ev) => {
				// Ignore clicks on action buttons.
				if ((ev.target as HTMLElement).closest(".result-actions")) return;
				this.select(i);
			});
			li.addEventListener("dblclick", (ev) => {
				if ((ev.target as HTMLElement).closest(".result-actions")) return;
				this.select(i);
				void this.open(item);
			});
			li.addEventListener("keydown", (ev) => {
				if (ev.key === "Enter") {
					ev.preventDefault();
					this.select(i);
					void this.open(item);
				}
			});

			if (this.showActions) {
				li.appendChild(this.buildActions(item));
			}

			this.list.appendChild(li);
		}
	}

	private buildActions(item: SearchResultItem): HTMLElement {
		const wrap = document.createElement("span");
		wrap.className = "result-actions";

		const mk = (label: string, title: string, fn: () => Promise<void>): HTMLButtonElement => {
			const b = document.createElement("button");
			b.type = "button";
			b.className = "icon-btn";
			b.textContent = label;
			b.title = title;
			b.addEventListener("click", (ev) => {
				ev.stopPropagation();
				fn().catch((e) => this.flashStatus(String(e)));
			});
			return b;
		};

		wrap.appendChild(
			mk("Open", "Open with default app", async () => {
				await this.open(item);
			}),
		);
		wrap.appendChild(
			mk("Reveal", "Show in folder", async () => {
				await revealItem(item.path);
			}),
		);
		wrap.appendChild(
			mk("Copy", "Copy path", async () => {
				await copyItemPath(item.path);
				this.flashStatus("Path copied.");
			}),
		);
		wrap.appendChild(
			mk("Copy file", "Copy file to clipboard", async () => {
				if (item.is_dir) {
					this.flashStatus("Folders can't be copied as files.");
					return;
				}
				await copyItemFile(item.path);
				this.flashStatus("File copied.");
			}),
		);
		wrap.appendChild(
			mk("Delete", "Move to recycle bin", async () => {
				if (!window.confirm(`Move to recycle bin?\n${item.path}`)) return;
				await deleteItem(item.path);
				// Remove from local list and re-render.
				this.rawItems = this.rawItems.filter((x) => x.path !== item.path);
				this.lastTotal = Math.max(0, this.lastTotal - 1);
				if (this.selected >= this.rawItems.length) this.selected = this.rawItems.length - 1;
				this.applySortAndRender();
				this.highlight();
				this.emitStatus();
				this.onSelect?.(this.current() ?? null);
			}),
		);
		return wrap;
	}
}
