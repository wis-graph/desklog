// active_window() 가 무엇을 돌려주는지 그대로 본다.
#[path = "../src/platform.rs"]
mod platform;

fn main() {
    for _ in 0..15 {
        let raw = active_win_pos_rs::get_active_window()
            .map(|w| format!("{:?}/{:?}", w.app_name, w.title))
            .unwrap_or_else(|_| "Err".into());
        println!(
            "잠김={} 유휴={:.0}s 크레이트={:<24} 최전면={:?}  →  {:?}",
            platform::screen_locked(),
            platform::idle_seconds(),
            raw,
            platform::frontmost_app(),
            platform::active_window()
        );
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
