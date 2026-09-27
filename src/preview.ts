// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

// Preview pane: file info for the currently selected search result.

import {
	type SearchResultItem,
	copyItemFile,
	copyItemPath,
	deleteItem,
	formatModified,
	formatSize,
	kindOf,
	openItem,
	revealItem,
} from "./shared";

export class Preview {
	private root: HTMLElement;

	constructor(root: HTMLElement) {
		this.root = root;
	}

	clear(): void {
		this.root.innerHTML = '<p class="preview-empty">Select a result to preview.</p>';
	}

	show(item: SearchResultItem | null): void {
		if (!item) {
			this.clear();
			return;
		}
		this.root.innerHTML = "";

		const title = document.createElement("div");
		title.className = "preview-title";
		title.textContent = item.name;

		const kind = document.createElement("div");
		kind.className = "preview-kind";
		kind.textContent = kindOf(item);

		const dl = document.createElement("dl");
		dl.className = "preview-dl";
		const rows: Array<[string, string]> = [
			["Path", item.path],
			["Size", item.is_dir ? "—" : formatSize(item.size)],
			["Modified", formatModified(item.modified_ms)],
			["Score", String(item.score)],
		];
		for (const [k, v] of rows) {
			const dt = document.createElement("dt");
			dt.textContent = k;
			const dd = document.createElement("dd");
			dd.textContent = v;
			dd.title = v;
			dl.append(dt, dd);
		}

		const actions = document.createElement("div");
		actions.className = "preview-actions";
		const mk = (label: string, fn: () => Promise<void>): HTMLButtonElement => {
			const b = document.createElement("button");
			b.type = "button";
			b.className = "secondary";
			b.textContent = label;
			b.addEventListener("click", () => {
				fn().catch((e) => {
					const err = this.root.querySelector(".preview-error");
					const msg = String(e);
					if (err) err.textContent = msg;
					else {
						const p = document.createElement("p");
						p.className = "preview-error";
						p.textContent = msg;
						this.root.appendChild(p);
					}
				});
			});
			return b;
		};
		actions.append(
			mk("Open", () => openItem(item.path)),
			mk("Reveal", () => revealItem(item.path)),
			mk("Copy path", async () => {
				await copyItemPath(item.path);
			}),
			mk("Copy file", async () => {
				if (item.is_dir) return;
				await copyItemFile(item.path);
			}),
			mk("Delete", async () => {
				if (!window.confirm(`Move to recycle bin?\n${item.path}`)) return;
				await deleteItem(item.path);
				this.clear();
			}),
		);

		this.root.append(title, kind, dl, actions);
	}
}
