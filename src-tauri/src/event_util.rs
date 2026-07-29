//! Emit a global Tauri event SAFELY from any thread.
//!
//! Calling [`tauri::Emitter::emit`] directly from a BACKGROUND thread holds
//! Tauri's webviews mutex while blocking on the main thread to run the delivery
//! JS (`Webview::eval`). If the main thread is concurrently handling an IPC call
//! that needs that same mutex (`AppManager::get_webview`), the two wait on each
//! other forever — a real UI freeze seen on this app (a `fetch` streaming
//! `sync-progress` while the file-watcher's `repo-changed` flood drove
//! main-thread IPC; caught by a live `sample` of the frozen process). See the
//! graph-batch fix (commit f72549c) for the same bug class, solved THERE with an
//! `ipc::Channel` because that stream is very high-frequency.
//!
//! `menu.rs` emits `menu-action` from the MAIN thread and never deadlocks, which
//! proves emitting ON the main thread is safe — the blocking `eval` path only
//! happens for an OFF-main-thread emit. So this marshals the emit onto the main
//! thread via [`tauri::AppHandle::run_on_main_thread`]: the background caller
//! returns immediately holding NO lock, and the emit runs inline on the main
//! thread, taking and releasing the webviews mutex within one event-loop turn —
//! never across a thread hop, so it cannot deadlock. Every background-thread
//! event in this app (`sync-progress`, `bisect-run-progress`, `repo-changed`,
//! `terminal-output`/`terminal-exit`) goes through here; only the very hot
//! `graph-batch` stream uses an `ipc::Channel` instead.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Wry};

/// Emit `event` with `payload` to all listeners, always from the main thread
/// (see the module doc for why direct off-thread `emit` deadlocks). Fire and
/// forget: a failed marshal/emit is dropped, matching every call site's existing
/// `let _ = app.emit(...)`.
pub fn emit_on_main<S: Serialize + Clone + Send + 'static>(app: &AppHandle<Wry>, event: &'static str, payload: S) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = handle.emit(event, payload);
    });
}
