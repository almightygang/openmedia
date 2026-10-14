import React from "react";

const INTERACTIVE_SELECTOR =
  "button, a, input, select, textarea, [contenteditable='true'], [data-no-drag]";

/**
 * Manual titlebar-drag handler (fallback for `data-tauri-drag-region`,
 * which some Linux WebViews ignore). Attach to the titlebar root's
 * `onMouseDown`: a primary-button press on a non-interactive area starts a
 * native window drag. Clicks on buttons/inputs/links are left alone.
 */
export function handleTitlebarMouseDown(event: React.MouseEvent<HTMLElement>): void {
  if (event.button !== 0) return;
  const target = event.target as HTMLElement | null;
  if (target && typeof target.closest === "function" && target.closest(INTERACTIVE_SELECTOR)) {
    return;
  }
  void window.windowControls.startDragging().catch(() => {
    // Best-effort: a failed drag must never break clicks.
  });
}
