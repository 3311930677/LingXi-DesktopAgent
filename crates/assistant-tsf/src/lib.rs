//! LingXi TSF Text Input Processor (TIP) — system-level pinyin IME.
//!
//! A COM DLL that registers as a Windows input method. Intercepts keystrokes,
//! drives the `assistant-ime` engine, and manages inline composition text.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::TextServices::*;

use assistant_ime::{InputContext, InputEngine, PinyinInputEngine};

// ─── GUIDs ──────────────────────────────────────────────────────────────────

const CLSID_LINGXI_TIP: GUID = GUID::from_u128(0x7E4B1D30_A1F2_4C5D_B8E6_9F0A2D3C4E5B);

#[allow(dead_code)]
const GUID_LINGXI_PROFILE: GUID = GUID::from_u128(0xA2B3C4D5_E6F7_0819_2A3B_4C5D6E7F8091);

#[allow(dead_code)]
const LANGID_ZH_CN: u16 = 0x0804;

// ─── Globals ────────────────────────────────────────────────────────────────

static DLL_INSTANCE: AtomicIsize = AtomicIsize::new(0);
static LOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
static OBJ_COUNT: AtomicUsize = AtomicUsize::new(0);

// ─── DLL entry ──────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
extern "system" fn DllMain(hinst: HMODULE, reason: u32, _: *mut ()) -> BOOL {
    if reason == 1 {
        DLL_INSTANCE.store(hinst.0 as isize, Ordering::Relaxed);
    }
    TRUE
}

// ─── DLL exports ────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut std::ffi::c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *ppv = std::ptr::null_mut() };
    if unsafe { *rclsid } != CLSID_LINGXI_TIP {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = ClassFactory.into();
    // Use the windows-core interface cast to get the requested IID.
    match factory.cast::<IUnknown>() {
        Ok(unk) => {
            unsafe { *ppv = std::mem::transmute(unk) };
            S_OK
        }
        Err(e) => e.code(),
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJ_COUNT.load(Ordering::Acquire) == 0 && LOCK_COUNT.load(Ordering::Acquire) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllRegisterServer() -> HRESULT {
    S_OK
}

#[unsafe(no_mangle)]
extern "system" fn DllUnregisterServer() -> HRESULT {
    S_OK
}

// ─── Class Factory ──────────────────────────────────────────────────────────

#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        _outer: Option<&IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut std::ffi::c_void,
    ) -> Result<()> {
        if ppv.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *ppv = std::ptr::null_mut() };
        let service = LingXiTextService::new();
        let unk: IUnknown = service.into();
        unsafe { *ppv = std::mem::transmute(unk) };
        Ok(())
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        if flock.as_bool() {
            LOCK_COUNT.fetch_add(1, Ordering::AcqRel);
        } else {
            LOCK_COUNT.fetch_sub(1, Ordering::AcqRel);
        }
        Ok(())
    }
}

// ─── Text Service ───────────────────────────────────────────────────────────

/// TSF TIP runs in an STA — all methods are called on the same thread. We use
/// interior mutability without Mutex because the COM apartment guarantees no
/// concurrent access, and `#[implement]` demands Send+Sync on the struct.
///
/// The only mutable state is the pinyin buffer; the engine is read-only after
/// construction.
#[implement(ITfTextInputProcessorEx, ITfKeyEventSink)]
struct LingXiTextService {
    engine: PinyinInputEngine,
    client_id: AtomicUsize,
    // Pinyin buffer and composing flag are only touched from the STA thread.
    // Using a raw pointer to a heap-allocated buffer to satisfy Send+Sync.
    buf: std::sync::Mutex<ImeBuffer>,
}

#[derive(Default)]
struct ImeBuffer {
    pinyin: String,
    composing: bool,
}

// ImeBuffer is only accessed from the STA thread; the Mutex is just to satisfy
// the compiler's Send+Sync requirements of #[implement].
unsafe impl Send for ImeBuffer {}
unsafe impl Sync for ImeBuffer {}

impl LingXiTextService {
    fn new() -> Self {
        OBJ_COUNT.fetch_add(1, Ordering::AcqRel);
        Self {
            engine: PinyinInputEngine::builtin(),
            client_id: AtomicUsize::new(0),
            buf: std::sync::Mutex::new(ImeBuffer::default()),
        }
    }
}

impl Drop for LingXiTextService {
    fn drop(&mut self) {
        OBJ_COUNT.fetch_sub(1, Ordering::AcqRel);
    }
}

// ─── ITfTextInputProcessorEx / ITfTextInputProcessor ────────────────────────

impl ITfTextInputProcessorEx_Impl for LingXiTextService_Impl {
    fn ActivateEx(&self, _ptim: Option<&ITfThreadMgr>, tid: u32, _flags: u32) -> Result<()> {
        self.client_id.store(tid as usize, Ordering::Relaxed);
        // TODO: AdviseKeyEventSink, AdviseCompositionSink
        Ok(())
    }
}

impl ITfTextInputProcessor_Impl for LingXiTextService_Impl {
    fn Activate(&self, ptim: Option<&ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        let mut buf = self.buf.lock().unwrap();
        buf.pinyin.clear();
        buf.composing = false;
        Ok(())
    }
}

// ─── ITfKeyEventSink ────────────────────────────────────────────────────────

impl ITfKeyEventSink_Impl for LingXiTextService_Impl {
    fn OnSetFocus(&self, _fforeground: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnTestKeyDown(
        &self,
        _pic: Option<&ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = wparam.0 as u16;
        let buf = self.buf.lock().unwrap();
        let eat = is_alpha_key(vk) || (buf.composing && is_ime_key(vk));
        Ok(BOOL::from(eat))
    }

    fn OnTestKeyUp(
        &self,
        _pic: Option<&ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(FALSE)
    }

    fn OnKeyDown(
        &self,
        _pic: Option<&ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let vk = wparam.0 as u16;
        let mut buf = self.buf.lock().unwrap();

        if is_alpha_key(vk) {
            buf.pinyin.push((vk as u8 as char).to_ascii_lowercase());
            buf.composing = true;
            // TODO: update inline composition text
            return Ok(TRUE);
        }
        if !buf.composing {
            return Ok(FALSE);
        }

        match vk {
            k if k == VK_SPACE.0 || k == VK_RETURN.0 => {
                let cands = self
                    .engine
                    .candidates(&buf.pinyin, &InputContext::with_limit(9));
                if let Some(top) = cands.first() {
                    // TODO: commit top.text via ITfComposition
                    let _ = &top.text;
                }
                buf.pinyin.clear();
                buf.composing = false;
                Ok(TRUE)
            }
            k if (0x31..=0x39).contains(&k) => {
                let idx = (k - 0x31) as usize;
                let cands = self
                    .engine
                    .candidates(&buf.pinyin, &InputContext::with_limit(9));
                if idx < cands.len() {
                    let _ = &cands[idx].text;
                    // TODO: commit
                }
                buf.pinyin.clear();
                buf.composing = false;
                Ok(TRUE)
            }
            k if k == VK_BACK.0 => {
                buf.pinyin.pop();
                if buf.pinyin.is_empty() {
                    buf.composing = false;
                }
                Ok(TRUE)
            }
            k if k == VK_ESCAPE.0 => {
                buf.pinyin.clear();
                buf.composing = false;
                Ok(TRUE)
            }
            _ => Ok(FALSE),
        }
    }

    fn OnKeyUp(
        &self,
        _pic: Option<&ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(FALSE)
    }

    fn OnPreservedKey(&self, _pic: Option<&ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(FALSE)
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn is_alpha_key(vk: u16) -> bool {
    (0x41..=0x5A).contains(&vk)
}

fn is_ime_key(vk: u16) -> bool {
    is_alpha_key(vk)
        || vk == VK_SPACE.0
        || vk == VK_RETURN.0
        || vk == VK_BACK.0
        || vk == VK_ESCAPE.0
        || (0x31..=0x39).contains(&vk)
}
