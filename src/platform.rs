//! OS별로 갈리는 유일한 곳. 나머지 코드는 이 두 함수만 본다.

/// 활성 창의 (앱 이름, 창 제목). 제목은 못 읽을 수 있다.
/// macOS는 다른 앱의 창 제목에 화면 기록 권한이 필요해서, 미승인이면 None이 온다.
pub fn active_window() -> Option<(String, Option<String>)> {
    let (app, title) = match active_win_pos_rs::get_active_window() {
        Ok(w) => (w.app_name.trim().to_string(), non_empty(w.title)),
        // 최전면 앱에 열거 가능한 창이 없으면 통째로 실패한다. 카카오톡 대화창에서 실제로 그렇다.
        Err(_) => (String::new(), None),
    };
    // 창을 못 읽어도 최전면 앱 이름은 따로 물어볼 수 있다.
    let app = if app.is_empty() {
        frontmost_app()?
    } else {
        app
    };
    Some((app, title))
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 창이 아니라 앱을 묻는다. 창 조회가 실패해도 이쪽은 답한다.
#[cfg(target_os = "macos")]
pub fn frontmost_app() -> Option<String> {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        if ws.is_null() {
            return None;
        }
        let app: *mut Object = msg_send![ws, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let name: *mut Object = msg_send![app, localizedName];
        if name.is_null() {
            return None;
        }
        let utf8: *const std::os::raw::c_char = msg_send![name, UTF8String];
        if utf8.is_null() {
            return None;
        }
        let s = std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned();
        non_empty(s)
    }
}

/// 윈도우에서는 창 조회가 실패하는 경우를 아직 보지 못했다. 보이면 그때 채운다.
#[cfg(not(target_os = "macos"))]
pub fn frontmost_app() -> Option<String> {
    None
}

/// 마지막 키보드·마우스 입력 이후 경과 초.
/// 입력 내용은 읽지 않는다. 훅도 걸지 않는다 — 물어보기만 한다.
#[cfg(target_os = "macos")]
pub fn idle_seconds() -> f64 {
    const COMBINED_SESSION_STATE: u32 = 0;
    const ANY_INPUT_EVENT: u32 = 0xFFFF_FFFF;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: u32, event_type: u32) -> f64;
    }
    unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT) }
}

#[cfg(windows)]
pub fn idle_seconds() -> f64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        if GetLastInputInfo(&mut info) == 0 {
            return 0.0;
        }
        GetTickCount().wrapping_sub(info.dwTime) as f64 / 1000.0
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn idle_seconds() -> f64 {
    0.0
}
