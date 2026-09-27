// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

import "./styles.css";
import { isPopupWindow } from "./shared";

// Route to popup or main window UI based on ?window=popup query param.
if (isPopupWindow()) {
	void import("./popup");
} else {
	void import("./main-window");
}
