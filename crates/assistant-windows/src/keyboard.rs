//! Minimal, auditable keyboard injection used only after target validation.

use std::mem::size_of;
use std::thread::sleep;
use std::time::{Duration, Instant};

use assistant_core::AdapterError;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_DELETE, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
    VK_A, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V, VK_Z,
};

/// Modifier keys that could still be physically held when a hotkey fires and
/// would otherwise contaminate our injected shortcuts (e.g. a lingering Alt
/// turning an injected Ctrl+V into Ctrl+Alt+V and triggering a third-party
/// global hotkey such as QQ's panel).
const STRAY_MODIFIERS: [VIRTUAL_KEY; 11] = [
    VK_CONTROL,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_MENU,
    VK_LMENU,
    VK_RMENU,
    VK_SHIFT,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LWIN,
    VK_RWIN,
];

/// Synthesize key-up events for every modifier so the following injection is a
/// clean chord regardless of what the user is still physically pressing. The
/// OS will emit its own key-up when the physical key is eventually released,
/// which is harmless.
fn release_stray_modifiers() -> Result<(), AdapterError> {
    let inputs: Vec<INPUT> = STRAY_MODIFIERS
        .iter()
        .map(|key| key_input(*key, 0, KEYEVENTF_KEYUP))
        .collect();
    send_all(&inputs)?;
    // Give the target a moment to process the key-ups before the real chord.
    sleep(Duration::from_millis(15));
    Ok(())
}

/// Wait until the trigger chord is physically released. `WM_HOTKEY` can arrive
/// while Ctrl/Alt are still held; injecting another shortcut before release
/// would create an unintended Ctrl+Alt combination in the target app.
pub fn wait_for_trigger_release(timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: key-state queries have no memory preconditions.
        let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0;
        // SAFETY: key-state queries have no memory preconditions.
        let alt = unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } < 0;
        if !ctrl && !alt {
            return;
        }
        sleep(Duration::from_millis(5));
    }
}

pub(crate) fn copy() -> Result<(), AdapterError> {
    send_ctrl_key(VK_C)
}

pub(crate) fn paste() -> Result<(), AdapterError> {
    send_ctrl_key(VK_V)
}

pub(crate) fn select_all() -> Result<(), AdapterError> {
    send_ctrl_key(VK_A)
}

pub(crate) fn undo() -> Result<(), AdapterError> {
    send_ctrl_key(VK_Z)
}

pub(crate) fn delete_selection() -> Result<(), AdapterError> {
    release_stray_modifiers()?;
    let inputs = [
        key_input(VK_DELETE, 0, Default::default()),
        key_input(VK_DELETE, 0, KEYEVENTF_KEYUP),
    ];
    send_all(&inputs)
}

/// Type arbitrary UTF-16 text into the active selection using KEYEVENTF_UNICODE.
pub fn type_unicode(text: &str) -> Result<(), AdapterError> {
    release_stray_modifiers()?;
    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for code_unit in text.encode_utf16() {
        inputs.push(key_input(VIRTUAL_KEY(0), code_unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(
            VIRTUAL_KEY(0),
            code_unit,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }
    send_all(&inputs)
}

fn send_ctrl_key(key: VIRTUAL_KEY) -> Result<(), AdapterError> {
    // Clear any physically-held modifiers first, then press only the Ctrl we
    // need, so the target sees exactly Ctrl+<key> and nothing else.
    release_stray_modifiers()?;
    let inputs = [
        key_input(VK_CONTROL, 0, Default::default()),
        key_input(key, 0, Default::default()),
        key_input(key, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];
    send_all(&inputs)
}

fn key_input(
    virtual_key: VIRTUAL_KEY,
    scan_code: u16,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_all(inputs: &[INPUT]) -> Result<(), AdapterError> {
    if inputs.is_empty() {
        return Ok(());
    }
    // SAFETY: `inputs` is a valid contiguous INPUT array and cbSize matches.
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(AdapterError::Platform(format!(
            "SendInput sent {sent}/{} events",
            inputs.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn utf16_event_count_handles_surrogate_pairs() {
        let text = "A中😀";
        assert_eq!(text.encode_utf16().count(), 4);
    }
}
