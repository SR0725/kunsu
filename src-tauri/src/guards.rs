//! GuardProbe 真偵測（CONTRACT §3 Guards / BDD forcing-and-guards）。
//!
//! 每項獨立 try、失敗=false+log，**絕不 panic**——守門寧嚴勿鬆的例外方向是
//! 「偵測失敗時不擋強制」（false），因為 guard 的語義是「確認有會議/錄影才擋」；
//! 但任何偵測錯誤都要留 log 可稽核。
//!
//! - 攝影機：CoreMediaIO `kCMIODevicePropertyDeviceIsRunningSomewhere`（'gone'）
//! - 麥克風：CoreAudio 同名屬性，僅檢查**有輸入流**的裝置（避免喇叭播放誤判）
//! - 螢幕擷取：`CGDisplayIsCaptured`（只在有 app 獨佔擷取整個螢幕時成立）。已知限制（BDD edge-notch 場景 12，
//!   2026-10-09 評估）：一般螢幕錄影與 ScreenCaptureKit 分享偵測不到——macOS 沒有公開 API 回報別的 app 正在擷取
//!   螢幕，選單列錄影指示沒有對外介面，用執行中的 app 名稱猜測會把「開著 OBS／QuickTime」誤判為錄影。
//! - 前景全螢幕：main-thread 輪詢（NSWorkspace bundle id 黑名單 + CGWindowList
//!   前景視窗蓋滿主螢幕判定）寫入 AtomicBool，本 probe 只讀——因為 AppKit 查詢
//!   不保證能在 timer 執行緒安全呼叫。
//! - dnd：本波不實作（Focus 狀態無公開 API），恆 false；quiet hours/kill-switch
//!   由 policy/service 判（CONTRACT §3）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use kunsu_server::{GuardProbe, GuardSnapshot};

/// 前景全螢幕黑名單（任務指定）。
pub const FULLSCREEN_BLACKLIST: [&str; 5] = [
    "zoom.us",
    "us.zoom.xos",
    "com.obsproject.obs-studio",
    "com.apple.iWork.Keynote",
    "com.microsoft.teams",
];

/// main-thread 輪詢與 timer 執行緒共用的旗標。
#[derive(Default)]
pub struct SharedGuardFlags {
    pub frontmost_fullscreen: AtomicBool,
}

pub struct MacGuardProbe {
    shared: Arc<SharedGuardFlags>,
}

impl MacGuardProbe {
    pub fn new(shared: Arc<SharedGuardFlags>) -> Self {
        MacGuardProbe { shared }
    }
}

impl GuardProbe for MacGuardProbe {
    fn snapshot(&self) -> GuardSnapshot {
        GuardSnapshot {
            screen_shared: probe("screen_shared", ffi::screen_captured),
            camera_on: probe("camera_on", ffi::camera_running_somewhere),
            mic_on: probe("mic_on", ffi::mic_running_somewhere),
            frontmost_fullscreen: self.shared.frontmost_fullscreen.load(Ordering::Relaxed),
            dnd: false,
        }
    }
}

/// 每項獨立 try：Err → false + log；panic → catch_unwind 吸收 → false + log。
fn probe(name: &str, f: fn() -> Result<bool, String>) -> bool {
    match std::panic::catch_unwind(f) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::warn!("guard probe {name} 失敗（視為 false）：{e}");
            false
        }
        Err(_) => {
            tracing::error!("guard probe {name} panic（已吸收，視為 false）");
            false
        }
    }
}

/// macOS FFI（CoreMediaIO / CoreAudio / CoreGraphics）。
#[cfg(target_os = "macos")]
pub mod ffi {
    use std::ffi::c_void;

    const fn fourcc(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    // ---- 共用 property address（CMIO 與 CoreAudio 同構）----
    #[repr(C)]
    struct PropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const SEL_DEVICES: u32 = fourcc(b"dev#");
    const SEL_RUNNING_SOMEWHERE: u32 = fourcc(b"gone");
    const SCOPE_GLOBAL: u32 = fourcc(b"glob");
    const SCOPE_WILDCARD: u32 = fourcc(b"****");
    const SCOPE_INPUT: u32 = fourcc(b"inpt");
    const SEL_STREAMS: u32 = fourcc(b"stm#");
    const ELEMENT_MAIN: u32 = 0;
    const ELEMENT_WILDCARD: u32 = u32::MAX;

    #[link(name = "CoreMediaIO", kind = "framework")]
    unsafe extern "C" {
        fn CMIOObjectGetPropertyDataSize(
            object_id: u32,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            out_size: *mut u32,
        ) -> i32;
        fn CMIOObjectGetPropertyData(
            object_id: u32,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            data_size: u32,
            data_used: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object_id: u32,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            out_size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            object_id: u32,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            data_size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsCaptured(display: u32) -> u32; // deprecated；任務指名可用
    }

    const CMIO_SYSTEM_OBJECT: u32 = 1;
    const AUDIO_SYSTEM_OBJECT: u32 = 1;

    /// 螢幕擷取（CGDisplayIsCaptured）。注意：偵測的是 display capture 模式；
    /// 視訊會議的視窗級分享偵測屬 SCK 等價實作，列人工驗收清單。
    pub fn screen_captured() -> Result<bool, String> {
        Ok(unsafe { CGDisplayIsCaptured(CGMainDisplayID()) } != 0)
    }

    /// 攝影機：任一 CMIO 裝置 DeviceIsRunningSomewhere。
    pub fn camera_running_somewhere() -> Result<bool, String> {
        let devices = cmio_devices()?;
        // 屬性位址逐組退避：不同 macOS 版本對 scope/element 的接受度不同。
        let addresses = [
            PropertyAddress {
                selector: SEL_RUNNING_SOMEWHERE,
                scope: SCOPE_WILDCARD,
                element: ELEMENT_WILDCARD,
            },
            PropertyAddress {
                selector: SEL_RUNNING_SOMEWHERE,
                scope: SCOPE_GLOBAL,
                element: ELEMENT_MAIN,
            },
        ];
        for dev in devices {
            for addr in &addresses {
                let mut val: u32 = 0;
                let mut used: u32 = 0;
                let status = unsafe {
                    CMIOObjectGetPropertyData(
                        dev,
                        addr,
                        0,
                        std::ptr::null(),
                        std::mem::size_of::<u32>() as u32,
                        &mut used,
                        (&raw mut val).cast(),
                    )
                };
                if status == 0 {
                    if val != 0 {
                        return Ok(true);
                    }
                    break; // 這台裝置查成功且沒在跑 → 換下一台
                }
            }
        }
        Ok(false)
    }

    fn cmio_devices() -> Result<Vec<u32>, String> {
        let addr = PropertyAddress {
            selector: SEL_DEVICES,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            CMIOObjectGetPropertyDataSize(
                CMIO_SYSTEM_OBJECT,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
            )
        };
        if status != 0 {
            return Err(format!("CMIO devices size 查詢失敗（OSStatus {status}）"));
        }
        let count = size as usize / std::mem::size_of::<u32>();
        let mut ids = vec![0u32; count];
        let mut used: u32 = 0;
        let status = unsafe {
            CMIOObjectGetPropertyData(
                CMIO_SYSTEM_OBJECT,
                &addr,
                0,
                std::ptr::null(),
                size,
                &mut used,
                ids.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            return Err(format!("CMIO devices 讀取失敗（OSStatus {status}）"));
        }
        ids.truncate(used as usize / std::mem::size_of::<u32>());
        Ok(ids)
    }

    /// 麥克風：只看「有輸入流」的 CoreAudio 裝置的 DeviceIsRunningSomewhere
    /// ——輸出裝置播音樂也會 running，必須排除。
    pub fn mic_running_somewhere() -> Result<bool, String> {
        let devices = audio_devices()?;
        for dev in devices {
            if !audio_has_input_streams(dev) {
                continue;
            }
            let addr = PropertyAddress {
                selector: SEL_RUNNING_SOMEWHERE,
                scope: SCOPE_GLOBAL,
                element: ELEMENT_MAIN,
            };
            let mut val: u32 = 0;
            let mut size: u32 = std::mem::size_of::<u32>() as u32;
            let status = unsafe {
                AudioObjectGetPropertyData(
                    dev,
                    &addr,
                    0,
                    std::ptr::null(),
                    &mut size,
                    (&raw mut val).cast(),
                )
            };
            if status == 0 && val != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn audio_devices() -> Result<Vec<u32>, String> {
        let addr = PropertyAddress {
            selector: SEL_DEVICES,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                AUDIO_SYSTEM_OBJECT,
                &addr,
                0,
                std::ptr::null(),
                &mut size,
            )
        };
        if status != 0 {
            return Err(format!("CoreAudio devices size 查詢失敗（OSStatus {status}）"));
        }
        let count = size as usize / std::mem::size_of::<u32>();
        let mut ids = vec![0u32; count];
        let mut got = size;
        let status = unsafe {
            AudioObjectGetPropertyData(
                AUDIO_SYSTEM_OBJECT,
                &addr,
                0,
                std::ptr::null(),
                &mut got,
                ids.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            return Err(format!("CoreAudio devices 讀取失敗（OSStatus {status}）"));
        }
        ids.truncate(got as usize / std::mem::size_of::<u32>());
        Ok(ids)
    }

    fn audio_has_input_streams(dev: u32) -> bool {
        let addr = PropertyAddress {
            selector: SEL_STREAMS,
            scope: SCOPE_INPUT,
            element: ELEMENT_MAIN,
        };
        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(dev, &addr, 0, std::ptr::null(), &mut size)
        };
        status == 0 && size as usize >= std::mem::size_of::<u32>()
    }

    // ---- 前景全螢幕（main thread 輪詢用；CoreFoundation/CGWindowList）----

    type CFTypeRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFStringRef = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct CGPoint {
        pub x: f64,
        pub y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct CGSize {
        pub width: f64,
        pub height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct CGRect {
        pub origin: CGPoint,
        pub size: CGSize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, index: isize) -> *const c_void;
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, number_type: isize, out: *mut c_void) -> bool;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
        fn CGRectMakeWithDictionaryRepresentation(
            dict: CFDictionaryRef,
            rect: *mut CGRect,
        ) -> bool;
        fn CGDisplayBounds(display: u32) -> CGRect;
        static kCGWindowLayer: CFStringRef;
        static kCGWindowOwnerPID: CFStringRef;
        static kCGWindowBounds: CFStringRef;
    }

    const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
    const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const K_CF_NUMBER_SINT64: isize = 4;

    fn dict_i64(dict: CFDictionaryRef, key: CFStringRef) -> Option<i64> {
        let v = unsafe { CFDictionaryGetValue(dict, key) };
        if v.is_null() {
            return None;
        }
        let mut out: i64 = 0;
        let ok = unsafe { CFNumberGetValue(v, K_CF_NUMBER_SINT64, (&raw mut out).cast()) };
        ok.then_some(out)
    }

    /// 指定 pid 的前景 layer-0 視窗是否蓋滿主螢幕（**只能在 main thread 呼叫
    /// AppKit 前先取好 pid**；本函數本身純 CF/CG，執行緒安全）。
    pub fn pid_has_fullscreen_window(front_pid: i32) -> Result<bool, String> {
        let main = unsafe { CGDisplayBounds(CGMainDisplayID()) };
        let list = unsafe {
            CGWindowListCopyWindowInfo(
                K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
                0,
            )
        };
        if list.is_null() {
            return Err("CGWindowListCopyWindowInfo 回 NULL（可能缺螢幕權限）".to_string());
        }
        let mut found = false;
        let count = unsafe { CFArrayGetCount(list) };
        for i in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(list, i) };
            if dict.is_null() {
                continue;
            }
            let layer = dict_i64(dict, unsafe { kCGWindowLayer }).unwrap_or(-1);
            let pid = dict_i64(dict, unsafe { kCGWindowOwnerPID }).unwrap_or(-1);
            if layer != 0 || pid != front_pid as i64 {
                continue;
            }
            let bounds_dict = unsafe { CFDictionaryGetValue(dict, kCGWindowBounds) };
            if bounds_dict.is_null() {
                continue;
            }
            let mut rect = CGRect::default();
            if !unsafe { CGRectMakeWithDictionaryRepresentation(bounds_dict, &mut rect) } {
                continue;
            }
            let eps = 1.0;
            if rect.origin.x <= main.origin.x + eps
                && rect.origin.y <= main.origin.y + eps
                && rect.size.width >= main.size.width - eps
                && rect.size.height >= main.size.height - eps
            {
                found = true;
                break;
            }
        }
        unsafe { CFRelease(list) };
        Ok(found)
    }
}

#[cfg(not(target_os = "macos"))]
pub mod ffi {
    pub fn screen_captured() -> Result<bool, String> {
        Ok(false)
    }
    pub fn camera_running_somewhere() -> Result<bool, String> {
        Ok(false)
    }
    pub fn mic_running_somewhere() -> Result<bool, String> {
        Ok(false)
    }
    pub fn pid_has_fullscreen_window(_pid: i32) -> Result<bool, String> {
        Ok(false)
    }
}
