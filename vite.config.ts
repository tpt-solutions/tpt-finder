import { defineConfig } from "vite";

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

export default defineConfig({
	clearScreen: false,
	server: {
		port: 1420,
		strictPort: true,
		watch: {
			ignored: ["**/src-tauri/**"],
		},
	},
	envPrefix: ["VITE_", "TAURI_"],
	build: {
		target: process.env.TAURI_PLATFORM === "windows" ? "chrome105" : "safari13",
		minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
		sourcemap: !!process.env.TAURI_DEBUG,
	},
});
