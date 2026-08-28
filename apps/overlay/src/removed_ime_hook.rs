/* IME mode removed: OwO is the only system input method. Keeping LingXi's former
WH_KEYBOARD_LL implementation here disabled would still risk accidental reactivation, so the
entire implementation is excluded and will be deleted after the migration remains stable.
本文件不参与编译：main.rs 中的 `#[cfg(any())] mod removed_ime_hook;` 使 rustc 跳过整个文件。
以下代码依赖已被删除的 assistant_ime crate，仅作迁移前参考，不可直接启用。 */
use super::*;
use std::sync::Arc;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW as GetMsg, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION,
    KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
};

/// Shared IME state between the hook thread, Tauri commands, and the frontend.
#[derive(Debug, Clone, Serialize)]
struct ImeStateView {
    active: bool,
    pinyin: String,
    candidates: Vec<ImeCandidateView>,
    /// `large` when connected to ime-server+rime-ice, `basic` on fallback.
    backend: &'static str,
}

#[derive(Debug, Serialize, Clone)]
struct ImeCandidateView {
    text: String,
    score: f64,
}

struct ImeShared {
    active: bool,
    pinyin: String,
    candidates: Vec<ImeCandidateView>,
    committed_context: String,
    /// Monotonic input revision. The worker computes candidates outside the
    /// hook and only publishes them if this revision is still current.
    revision: u64,
    /// Candidate selected by Space/Enter/1-9/click. The worker takes this and
    /// performs the clipboard paste outside the low-level hook callback.
    pending_commit: Option<String>,
    /// Selection pressed before the async candidate response arrived. The
    /// worker applies it as soon as the matching revision is published.
    pending_selection: Option<usize>,
    /// Whether the last candidate query came from ime-server+rime-ice.
    server_connected: bool,
}

impl Default for ImeShared {
    fn default() -> Self {
        Self {
            // Launch directly in Chinese input mode. Ctrl+Alt+I remains an
            // explicit toggle, but the first run no longer leaks raw pinyin.
            active: true,
            pinyin: String::new(),
            candidates: Vec::new(),
            committed_context: String::new(),
            revision: 0,
            pending_commit: None,
            pending_selection: None,
            server_connected: false,
        }
    }
}

static IME: std::sync::OnceLock<Arc<Mutex<ImeShared>>> = std::sync::OnceLock::new();

fn ime_shared() -> &'static Arc<Mutex<ImeShared>> {
    IME.get_or_init(|| Arc::new(Mutex::new(ImeShared::default())))
}

/// Poll IME state (called by the frontend every ~30ms).
#[tauri::command]
fn ime_state() -> ImeStateView {
    let s = ime_shared().safe_lock();
    ImeStateView {
        active: s.active,
        pinyin: s.pinyin.clone(),
        candidates: s.candidates.clone(),
        backend: if s.server_connected { "large" } else { "basic" },
    }
}

/// Queue a candidate chosen by mouse. Keyboard choices use the same queue from
/// the hook; the worker performs the actual paste without stealing focus.
#[tauri::command]
fn ime_commit(index: usize) -> Result<(), String> {
    let mut s = ime_shared().safe_lock();
    let text = s
        .candidates
        .get(index)
        .map(|candidate| candidate.text.clone())
        .ok_or("invalid candidate index")?;
    s.committed_context.push_str(&text);
    s.pending_commit = Some(text);
    s.pinyin.clear();
    s.candidates.clear();
    s.revision = s.revision.wrapping_add(1);
    Ok(())
}

/// Toggle IME mode on/off.
#[tauri::command]
fn ime_toggle() -> bool {
    let mut s = ime_shared().safe_lock();
    s.active = !s.active;
    if !s.active {
        s.pinyin.clear();
        s.candidates.clear();
        s.committed_context.clear();
        s.pending_commit = None;
        s.pending_selection = None;
    }
    s.revision = s.revision.wrapping_add(1);
    s.active
}

/// Compute candidates outside the keyboard hook. The caller publishes the
/// result only when the pinyin revision is still current.
fn compute_candidates(pinyin: &str, context: &str) -> (Vec<ImeCandidateView>, bool) {
    if let Some(results) = ipc_query(pinyin, context, 9) {
        return (results, true);
    }
    use assistant_ime::{InputContext, InputEngine, PinyinInputEngine};
    let engine = PinyinInputEngine::builtin();
    let ctx = InputContext {
        preceding_text: context.to_string(),
        max_candidates: 9,
    };
    let candidates = engine
        .candidates(pinyin, &ctx)
        .into_iter()
        .map(|candidate| ImeCandidateView {
            text: candidate.text,
            score: candidate.score,
        })
        .collect();
    (candidates, false)
}

/// TCP call to the ime-server.
fn ipc_query(pinyin: &str, context: &str, limit: usize) -> Option<Vec<ImeCandidateView>> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let mut stream = TcpStream::connect_timeout(
        &"127.0.0.1:9527".parse().unwrap(),
        std::time::Duration::from_millis(100),
    )
    .ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(500)))
        .ok()?;

    let request = format!(
        "{{\"type\":\"query\",\"pinyin\":\"{}\",\"context\":\"{}\",\"limit\":{}}}\n",
        pinyin.replace('\\', "\\\\").replace('"', "\\\""),
        context.replace('\\', "\\\\").replace('"', "\\\""),
        limit
    );
    stream.write_all(request.as_bytes()).ok()?;
    stream.flush().ok()?;

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;

    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let arr = value.get("candidates")?.as_array()?;
    let results = arr
        .iter()
        .filter_map(|item| {
            Some(ImeCandidateView {
                text: item.get("text")?.as_str()?.to_string(),
                score: item.get("score")?.as_f64().unwrap_or(0.0),
            })
        })
        .collect();
    Some(results)
}

/// Spawn the global low-level keyboard hook thread.
fn spawn_ime_hook_thread() {
    thread::spawn(move || {
        unsafe {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ime_hook_proc), None, 0)
                .expect("install keyboard hook");

            // Must pump messages to keep the hook alive.
            let mut msg = MSG::default();
            while GetMsg(&mut msg, None, 0, 0).0 > 0 {}

            let _ = UnhookWindowsHookEx(hook);
        }
    });
}

/// Slow IME worker: resolves candidates, commits text, and manages the panel.
/// Keeping all of this out of `ime_hook_proc` prevents Windows hook timeouts.
fn spawn_ime_worker(app: AppHandle) {
    let shared = ime_shared().clone();
    thread::spawn(move || {
        let mut seen_revision = u64::MAX;
        let mut visible = false;
        loop {
            let pending = shared.safe_lock().pending_commit.take();
            if let Some(text) = pending {
                if let Err(error) = insert_text_at_caret(&text) {
                    eprintln!("IME commit failed: {error}");
                }
            }

            let (active, revision, pinyin, context) = {
                let s = shared.safe_lock();
                (
                    s.active,
                    s.revision,
                    s.pinyin.clone(),
                    s.committed_context.clone(),
                )
            };

            if !active || pinyin.is_empty() {
                if visible {
                    if let Some(window) = app.get_webview_window("ime") {
                        let _ = window.hide();
                    }
                    visible = false;
                }
                seen_revision = revision;
            } else if revision != seen_revision {
                let (candidates, server_connected) = compute_candidates(&pinyin, &context);
                let mut s = shared.safe_lock();
                // Discard a stale server response if another key arrived.
                if s.active && s.revision == revision && s.pinyin == pinyin {
                    s.candidates = candidates;
                    s.server_connected = server_connected;
                    let queued = if let Some(index) = s.pending_selection.take() {
                        queue_candidate(&mut s, index);
                        s.pending_commit.is_some()
                    } else {
                        false
                    };
                    seen_revision = revision;
                    drop(s);
                    if !queued {
                        if let Some(window) = app.get_webview_window("ime") {
                            position_ime_window(&window);
                            let _ = window.show();
                            visible = true;
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
}

/// Position at the real application caret when available, otherwise at cursor.
fn position_ime_window(window: &WebviewWindow) {
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};

    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    let mut point = POINT::default();
    let caret_found =
        unsafe { GetGUIThreadInfo(0, &mut info) }.is_ok() && !info.hwndCaret.is_invalid() && {
            point.x = info.rcCaret.left;
            point.y = info.rcCaret.bottom;
            unsafe { ClientToScreen(info.hwndCaret, &mut point) }.as_bool()
        };
    if !caret_found && unsafe { GetCursorPos(&mut point) }.is_err() {
        return;
    }

    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return;
    }
    let work = info.rcWork;
    let size = window.outer_size().unwrap_or(PhysicalSize::new(720, 92));
    let x = point.x.min(work.right - size.width as i32).max(work.left);
    let y = (point.y + 8)
        .min(work.bottom - size.height as i32)
        .max(work.top);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// The low-level hook callback. It must return quickly: doing TCP or clipboard
/// work here makes Windows time out/remove the hook and leaks letters into the
/// target. This function only updates state; the worker does all slow work.
unsafe extern "system" fn ime_hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    if code as u32 != HC_ACTION {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    // Never intercept SendInput generated by our controlled paste; otherwise
    // the injected Ctrl+V would become another pinyin `v`.
    if kb.flags.0 & 0x10 != 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    // Preserve Ctrl/Alt/Win shortcuts, including Ctrl+Alt+I which toggles mode.
    let modifier_down = GetAsyncKeyState(VK_CONTROL.0 as i32) < 0
        || GetAsyncKeyState(VK_MENU.0 as i32) < 0
        || GetAsyncKeyState(VK_LWIN.0 as i32) < 0
        || GetAsyncKeyState(VK_RWIN.0 as i32) < 0;
    if modifier_down {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let shared = ime_shared();
    if !shared.safe_lock().active || wparam.0 != 0x0100 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let vk = kb.vkCode as u16;
    let handled = {
        let mut s = shared.safe_lock();
        if (0x41..=0x5A).contains(&vk) {
            s.pinyin.push((vk as u8 as char).to_ascii_lowercase());
            s.candidates.clear();
            s.pending_selection = None;
            s.revision = s.revision.wrapping_add(1);
            true
        } else if vk == VK_BACK.0 && !s.pinyin.is_empty() {
            s.pinyin.pop();
            s.candidates.clear();
            s.pending_selection = None;
            s.revision = s.revision.wrapping_add(1);
            true
        } else if vk == VK_ESCAPE.0 {
            // Escape always exits IME mode, even with an active composition.
            s.active = false;
            s.pinyin.clear();
            s.candidates.clear();
            s.committed_context.clear();
            s.pending_commit = None;
            s.pending_selection = None;
            s.revision = s.revision.wrapping_add(1);
            true
        } else if (vk == VK_SPACE.0 || vk == VK_RETURN.0) && !s.pinyin.is_empty() {
            if s.candidates.is_empty() {
                s.pending_selection = Some(0);
            } else {
                queue_candidate(&mut s, 0);
            }
            true
        } else if (0x31..=0x39).contains(&vk) && !s.pinyin.is_empty() {
            let index = (vk - 0x31) as usize;
            if s.candidates.is_empty() {
                s.pending_selection = Some(index);
            } else {
                queue_candidate(&mut s, index);
            }
            true
        } else {
            // Only eat non-letter keys while composing; ordinary keys pass
            // through when the buffer is empty.
            !s.pinyin.is_empty()
        }
    };

    if handled {
        windows::Win32::Foundation::LRESULT(1)
    } else {
        CallNextHookEx(None, code, wparam, lparam)
    }
}

fn queue_candidate(state: &mut ImeShared, index: usize) {
    if let Some(candidate) = state.candidates.get(index).cloned() {
        state.committed_context.push_str(&candidate.text);
        state.pending_commit = Some(candidate.text);
        state.pinyin.clear();
        state.candidates.clear();
        state.revision = state.revision.wrapping_add(1);
    }
}

fn on_ime(app: &AppHandle) {
    let active = {
        let mut s = ime_shared().safe_lock();
        s.active = !s.active;
        s.pinyin.clear();
        s.candidates.clear();
        s.pending_commit = None;
        s.pending_selection = None;
        if !s.active {
            s.committed_context.clear();
        }
        s.revision = s.revision.wrapping_add(1);
        s.active
    };
    // Activating mode does not show an empty panel; the worker shows it on the
    // first letter. Deactivation hides immediately.
    if !active {
        if let Some(window) = app.get_webview_window("ime") {
            let _ = window.hide();
        }
    }
}
