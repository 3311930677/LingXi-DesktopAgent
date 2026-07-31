//! QQ desktop integration through public UI Automation surfaces only.
//!
//! This is intentionally semi-automatic: it can detect a likely latest message
//! and place a user-approved draft into the edit control, but it has no "send"
//! operation. The user remains the final authority and presses Send in QQ.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread::sleep;
use std::time::Duration;

use assistant_core::AdapterError;
use windows::core::BSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{
    IUIAutomationElement, IUIAutomationValuePattern, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_ListItemControlTypeId, UIA_TextControlTypeId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::IsWindow;

use crate::clipboard;
use crate::com::ComGuard;
use crate::foreground::{foreground_info, window_info, ForegroundInfo};
use crate::keyboard;
use crate::read::{get_pattern, read_full_text};
use crate::uia::UiaClient;

const MAX_MESSAGE_CHARS: usize = 2_000;

/// Handle of the most recent foreground window that belonged to QQ.
///
/// The overlay's "read" and "write draft" buttons live on the LingXi panel, so
/// the moment the user clicks one, QQ is no longer the foreground window (the
/// panel is, especially now that the chat view intentionally takes keyboard
/// focus). Requiring "QQ is foreground right now" therefore made reading
/// impossible from the panel. Instead a background sampler records the last QQ
/// window here, and the QQ operations fall back to it when the live foreground
/// is something else (typically the panel itself). `0` means "unknown".
static LAST_QQ_HWND: AtomicIsize = AtomicIsize::new(0);

/// Record the foreground window if it currently belongs to QQ. Called
/// periodically by a lightweight background sampler so the panel can still act
/// on the user's QQ window after it steals focus.
pub fn remember_foreground_if_qq() {
    if let Ok(info) = foreground_info() {
        if is_qq_process(&info.process_name) {
            LAST_QQ_HWND.store(info.hwnd, Ordering::Relaxed);
        }
    }
}

/// Resolve the QQ window to operate on: the live foreground when it is QQ,
/// otherwise the last remembered QQ window if it still exists and is still QQ.
fn resolve_qq_window() -> Result<ForegroundInfo, AdapterError> {
    let foreground = foreground_info()?;
    if is_qq_process(&foreground.process_name) {
        LAST_QQ_HWND.store(foreground.hwnd, Ordering::Relaxed);
        return Ok(foreground);
    }

    let remembered = LAST_QQ_HWND.load(Ordering::Relaxed);
    if remembered != 0 {
        // SAFETY: `IsWindow` accepts any handle and simply reports validity.
        let alive = unsafe { IsWindow(HWND(remembered as *mut _)) }.as_bool();
        if alive {
            if let Some(info) = window_info(remembered) {
                if is_qq_process(&info.process_name) {
                    return Ok(info);
                }
            }
        }
        // Stale handle (QQ closed or reused): forget it so we stop pointing at
        // an unrelated window.
        LAST_QQ_HWND.store(0, Ordering::Relaxed);
    }

    Err(AdapterError::Platform(
        "QQ window was not found; open a QQ chat window first".into(),
    ))
}

/// Best-effort snapshot of the active QQ conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QqMessageSnapshot {
    pub hwnd: isize,
    pub conversation: String,
    pub message: String,
}

/// Read the most recent message-like text node from the foreground QQ window.
///
/// QQ is Chromium-based and its accessibility tree changes between versions,
/// so this deliberately uses semantic control types rather than private class
/// names. Failure is explicit; callers should keep polling instead of treating
/// unrelated foreground windows as conversations.
pub fn qq_latest_message() -> Result<QqMessageSnapshot, AdapterError> {
    let foreground = resolve_qq_window()?;
    let _com = ComGuard::new()?;
    let client = UiaClient::new()?;
    let root = client.element_from_hwnd(foreground.hwnd)?;
    let elements = client.descendants(&root)?;

    let mut candidates = Vec::new();
    for element in &elements {
        let control_type = match unsafe { element.CurrentControlType() } {
            Ok(value) => value,
            Err(_) => continue,
        };
        if control_type != UIA_TextControlTypeId
            && control_type != UIA_DocumentControlTypeId
            && control_type != UIA_ListItemControlTypeId
        {
            continue;
        }
        // Prefer the node's full text via TextPattern/ValuePattern: QQ is a
        // Chromium client whose message bubbles expose their complete content
        // there, while `Name` is frequently a truncated label. Fall back to
        // `Name` only when neither pattern is available.
        let text = element_text(element).unwrap_or_default();
        let text = text.trim();
        if is_message_candidate(text, &foreground.title) {
            candidates.push(text.to_string());
        }
    }

    // A single logical message is often split across several adjacent text
    // nodes in the Chromium accessibility tree (one per line/segment). Taking
    // only the last node would read just the final fragment ("a few chars").
    // Instead take a trailing run of candidates and join them so a multi-line
    // message is reconstructed whole.
    let message = join_trailing_message(&candidates)
        .ok_or_else(|| AdapterError::Platform("QQ message text was not exposed by UIA".into()))?;
    Ok(QqMessageSnapshot {
        hwnd: foreground.hwnd,
        conversation: foreground.title,
        message,
    })
}

/// Put a confirmed draft into the foreground QQ edit control without sending.
/// Existing input is replaced intentionally, and an exact read-back is required
/// when the provider exposes Value/TextPattern.
pub fn qq_write_draft(draft: &str) -> Result<bool, AdapterError> {
    if draft.trim().is_empty() {
        return Err(AdapterError::NoSelection);
    }
    let foreground = resolve_qq_window()?;
    let _com = ComGuard::new()?;
    let client = UiaClient::new()?;
    let root = client.element_from_hwnd(foreground.hwnd)?;
    let elements = client.descendants(&root)?;
    let editor = choose_editor(elements).ok_or(AdapterError::UnsupportedControl)?;

    // SAFETY: this is an enabled, keyboard-focusable Edit element in the
    // foreground QQ window. Focusing does not activate any other application.
    unsafe { editor.SetFocus() }
        .map_err(|error| AdapterError::Platform(format!("QQ editor SetFocus failed: {error}")))?;
    sleep(Duration::from_millis(80));

    if let Some(value) = get_pattern::<IUIAutomationValuePattern>(&editor, UIA_ValuePatternId) {
        // SAFETY: the provider advertises ValuePattern; it enforces read-only
        // state itself. This replaces the draft but never invokes Send.
        unsafe { value.SetValue(&BSTR::from(draft)) }.map_err(|error| {
            AdapterError::Platform(format!("QQ editor SetValue failed: {error}"))
        })?;
    } else {
        // Chromium editors often expose TextPattern only. Replace any existing
        // composer contents explicitly; this still stops before Send and is
        // verified exactly below when the provider allows reading the value.
        keyboard::select_all()?;
        clipboard::paste_text_preserving_clipboard(draft)?;
    }

    sleep(Duration::from_millis(120));
    Ok(read_full_text(&editor)?.is_some_and(|text| text == draft))
}

/// Full text of a single element, preferring the accessibility patterns that
/// expose complete content over `Name`.
///
/// `read_full_text` returns the whole `TextPattern.DocumentRange` (or the
/// `ValuePattern` value) when either is available; QQ's Chromium bubbles carry
/// their real content there. `Name` is used only as a last resort because it is
/// frequently a truncated label, which is what previously limited reads to "a
/// few characters".
fn element_text(element: &IUIAutomationElement) -> Option<String> {
    if let Ok(Some(text)) = read_full_text(element) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    unsafe { element.CurrentName() }
        .ok()
        .map(|value| value.to_string())
}

/// Reconstruct the latest logical message from an ordered list of candidate
/// text nodes.
///
/// Chromium often splits one bubble into several adjacent text nodes (one per
/// line). We therefore take a *trailing run* of candidates rather than just the
/// last node, joining them with newlines so a multi-line message is returned
/// whole. The run stops once it has gathered a reasonable amount of text so an
/// unrelated earlier bubble is not merged in.
fn join_trailing_message(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    // Walk backwards, accumulating fragments until we have collected enough
    // characters to plausibly cover one full message. Most fragments are short
    // lines; a handful of them reconstruct a normal chat message without
    // swallowing the entire scrollback.
    let mut collected: Vec<&str> = Vec::new();
    let mut total_chars = 0usize;
    for fragment in candidates.iter().rev() {
        let fragment = fragment.trim();
        if fragment.is_empty() {
            continue;
        }
        collected.push(fragment);
        total_chars += fragment.chars().count();
        // A single sufficiently long node is already a whole message; and once
        // the accumulated run is long enough, stop before reaching into older
        // bubbles.
        if total_chars >= 60 {
            break;
        }
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    let joined = collected.join("\n");
    let joined = joined.trim();
    if joined.is_empty() {
        None
    } else if joined.chars().count() > MAX_MESSAGE_CHARS {
        Some(joined.chars().take(MAX_MESSAGE_CHARS).collect())
    } else {
        Some(joined.to_string())
    }
}

/// Whether a process image name looks like a QQ client. Case-insensitive and
/// tolerant of the several executables QQ has shipped (classic `QQ.exe` and the
/// newer Electron `QQNT.exe`).
fn is_qq_process(process_name: &str) -> bool {
    let process = process_name.to_ascii_lowercase();
    process == "qq.exe" || process == "qqnt.exe"
}

fn is_message_candidate(text: &str, window_title: &str) -> bool {
    let text = text.trim();
    if text.is_empty()
        || text == window_title.trim()
        || text.chars().count() > MAX_MESSAGE_CHARS
        || matches!(
            text,
            "发送" | "表情" | "截图" | "文件" | "语音" | "视频" | "更多" | "搜索"
        )
    {
        return false;
    }
    // UI labels are usually one or two characters. Keeping at least one
    // alphanumeric/CJK character avoids punctuation-only timestamp separators.
    text.chars()
        .any(|ch| ch.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&ch))
}

fn choose_editor(elements: Vec<IUIAutomationElement>) -> Option<IUIAutomationElement> {
    let mut candidates = Vec::new();
    for element in elements {
        if unsafe { element.CurrentControlType() }.ok() != Some(UIA_EditControlTypeId) {
            continue;
        }
        if !unsafe { element.CurrentIsEnabled() }
            .ok()
            .is_some_and(|value| value.as_bool())
            || !unsafe { element.CurrentIsKeyboardFocusable() }
                .ok()
                .is_some_and(|value| value.as_bool())
            || unsafe { element.CurrentIsPassword() }
                .ok()
                .is_some_and(|value| value.as_bool())
        {
            continue;
        }
        let descriptor = format!(
            "{} {} {}",
            unsafe { element.CurrentName() }
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default(),
            unsafe { element.CurrentAutomationId() }
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default(),
            unsafe { element.CurrentClassName() }
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default()
        )
        .to_ascii_lowercase();
        let score = editor_score(&descriptor);
        let focused = unsafe { element.CurrentHasKeyboardFocus() }
            .ok()
            .is_some_and(|value| value.as_bool());
        // Vertical position tie-breaker: QQ's message composer always sits at
        // the bottom of the window while the search box is at the very top. When
        // neither editor holds focus (the usual case, since the LingXi panel
        // stole it) and keyword scores tie, the lower control is the composer.
        let bottom = unsafe { element.CurrentBoundingRectangle() }
            .map(|rect| rect.top)
            .unwrap_or(0);
        candidates.push((focused, score, bottom, element));
    }
    candidates
        .into_iter()
        .max_by_key(|(focused, score, bottom, _)| (*focused as usize, *score, *bottom))
        .map(|(_, _, _, element)| element)
}

/// Score an editor descriptor for how likely it is the chat composer rather
/// than an unrelated Edit control such as the top search box.
///
/// Search-related keywords are penalized so the top "搜索/search" field never
/// wins over the composer; composer keywords add points. The result is clamped
/// at zero so a strongly-search-flavored field cannot outrank an unlabeled but
/// otherwise valid composer.
fn editor_score(descriptor: &str) -> i32 {
    const COMPOSER_KEYWORDS: [&str; 7] = [
        "input", "editor", "compose", "message", "chat", "输入", "消息",
    ];
    const SEARCH_KEYWORDS: [&str; 3] = ["search", "搜索", "查找"];

    let positive = COMPOSER_KEYWORDS
        .into_iter()
        .filter(|keyword| descriptor.contains(keyword))
        .count() as i32;
    let negative = SEARCH_KEYWORDS
        .into_iter()
        .filter(|keyword| descriptor.contains(keyword))
        .count() as i32;
    // A search field is disqualified hard; a composer keyword adds a small
    // positive bias over an unlabeled control.
    positive - negative * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_candidate_filters_chrome_labels() {
        assert!(is_message_candidate("明天下午见吗？", "张三"));
        assert!(!is_message_candidate("发送", "张三"));
        assert!(!is_message_candidate("张三", "张三"));
        assert!(!is_message_candidate("...", "张三"));
    }

    #[test]
    fn join_trailing_message_merges_split_fragments() {
        // A multi-line message split across nodes is reconstructed whole
        // instead of returning only the final fragment.
        let candidates = vec![
            "早上好".to_string(),
            "今天".to_string(),
            "我们".to_string(),
            "去哪里".to_string(),
            "吃饭".to_string(),
        ];
        let joined = join_trailing_message(&candidates).unwrap();
        assert!(joined.contains("吃饭"));
        assert!(joined.contains("今天"));
        assert!(joined.chars().count() > "吃饭".chars().count());
    }

    #[test]
    fn join_trailing_message_returns_long_single_node_as_is() {
        let long = "这是一条很长的消息".repeat(10);
        let joined = join_trailing_message(std::slice::from_ref(&long)).unwrap();
        assert_eq!(joined, long);
    }

    #[test]
    fn join_trailing_message_none_when_empty() {
        assert!(join_trailing_message(&[]).is_none());
        assert!(join_trailing_message(&["".to_string(), "   ".to_string()]).is_none());
    }

    #[test]
    fn editor_score_penalizes_search_box() {
        // The top search field must never outrank the composer.
        assert!(editor_score("搜索 search-input searchbox") < 0);
        assert!(editor_score("消息输入 message-input composer") > 0);
        // A search-flavored control loses even to an unlabeled composer (0).
        assert!(editor_score("搜索") < editor_score("some-unlabeled-edit"));
    }

    #[test]
    fn editor_score_composer_beats_search() {
        assert!(editor_score("输入消息") > editor_score("搜索联系人"));
    }
}
