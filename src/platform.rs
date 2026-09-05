//! OS별로 갈리는 유일한 곳. 나머지 코드는 이 두 함수만 본다.

/// 활성 창의 (앱 이름, 창 제목). 제목은 못 읽을 수 있다.
/// macOS는 다른 앱의 창 제목에 화면 기록 권한이 필요해서, 미승인이면 None이 온다.
pub fn active_window() -> Option<(String, Option<String>)> {
    let w = active_win_pos_rs::get_active_window().ok()?;
    let title = if w.title.trim().is_empty() {
        None
    } else {
        Some(w.title)
    };
    let app = if w.app_name.trim().is_empty() {
        "unknown".to_string()
    } else {
        w.app_name
    };
    Some((app, title))
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
