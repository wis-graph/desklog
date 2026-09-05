//! desklog — 사용자가 무엇을 하고 있는지 기록하는 수집기.
//! 사용법은 `desklog --help` 또는 아래 HELP 상수를 본다.

mod platform;

use rusqlite::Connection;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 몇 초마다 한 줄 남길지. 상태는 수십 초 단위로 바뀌니 5초면 충분하다.
const SAMPLE_S: u64 = 5;
/// 이만큼 입력이 없으면 세션이 끊긴 것으로 본다.
const IDLE_BREAK_S: f64 = 300.0;
/// 이 안에 입력이 있었으면 '입력 중'으로 센다.
const ACTIVE_IDLE_S: f64 = 60.0;

const HELP: &str = "\
desklog — 사용자가 무엇을 하고 있는지 기록하는 수집기

사용법:
  desklog <명령> [인자]

명령:
  watch              상주하며 5초마다 기록한다. 재부팅하면 죽으니 다시 띄운다
  now                현재 상태를 JSON 한 줄로 출력한다 (로봇·스크립트가 호출)
  live               현재 상태를 1초마다 갱신한다
  log [개수]         원시 기록을 훑어본다 (기본 40). 끊긴 구간을 표시한다
  top [일수] [앱]    앱별 시간·시간대·창 제목 요약 (기본 7일, 앱을 주면 그 앱만)
  label yes|no       직전 개입이 먹혔는지 기록한다 (학습 라벨)
  export             학습용 CSV를 표준출력으로

옵션:
  -h, --help         이 도움말
  -v, --version      판 번호

상주시키기:
  nohup desklog watch > /tmp/desklog.log 2>&1 &

기록하는 것:
  활성 앱 이름, 창 제목, 마지막 입력 이후 경과 초, 지역시 '시',
  활동 세션 지속, 현재 앱 연속 사용 시간.

기록하지 않는 것:
  입력 내용, 화면 이미지, 네트워크 전송. 입력은 빈도만 세고 무엇을 눌렀는지 보지 않는다.

저장 위치:
  ~/.desklog.db (sqlite). 5초에 한 줄, 하루 1MB 미만.
";

fn main() {
    // Rust 런타임이 SIGPIPE를 무시로 바꿔놓아서 `desklog log | head` 가 패닉한다.
    // 기본 동작(조용히 종료)으로 되돌린다.
    #[cfg(unix)]
    unsafe {
        extern "C" {
            fn signal(sig: i32, handler: usize) -> usize;
        }
        const SIGPIPE: i32 = 13;
        const SIG_DFL: usize = 0;
        signal(SIGPIPE, SIG_DFL);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            print!("{HELP}");
            return;
        }
        Some("-v") | Some("--version") => {
            println!("desklog {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {}
    }
    let db = open_db();
    match args[0].as_str() {
        "watch" => watch(&db),
        "now" => println!("{}", now_json(&db)),
        "live" => live(&db),
        "log" => log(&db, args.get(1).and_then(|s| s.parse().ok()).unwrap_or(40)),
        "top" => top(
            &db,
            args.get(1).and_then(|s| s.parse().ok()).unwrap_or(7),
            args.get(2).map(String::as_str),
        ),
        "label" => label(&db, args.get(1).map(String::as_str)),
        "export" => export(&db),
        other => {
            eprintln!("모르는 명령: {other}\n");
            print!("{HELP}");
            std::process::exit(2);
        }
    }
}

// ---------- 저장 ----------

fn db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join(".desklog.db")
}

fn open_db() -> Connection {
    let db = Connection::open(db_path()).expect("db 열기 실패");
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS spans (
            id         INTEGER PRIMARY KEY,
            start_t    INTEGER NOT NULL,
            end_t      INTEGER NOT NULL,
            app        TEXT    NOT NULL,
            title      TEXT,
            hour       INTEGER NOT NULL,
            active_s   INTEGER NOT NULL,  -- 구간 안에서 입력이 있던 시간
            idle_s     REAL    NOT NULL,  -- 구간 끝 시점의 유휴 시간
            session_s  INTEGER NOT NULL,
            app_s      INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_spans_start ON spans(start_t);
         CREATE TABLE IF NOT EXISTS labels (
            t      INTEGER PRIMARY KEY,
            worked INTEGER NOT NULL
         );",
    )
    .expect("스키마 생성 실패");
    db
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// 로컬 시각의 '시'. 새벽인지 아닌지가 신호라서 UTC로는 못 쓴다.
fn local_hour(t: i64) -> i64 {
    let offset = local_utc_offset_seconds();
    (t + offset).rem_euclid(86400) / 3600
}

fn local_utc_offset_seconds() -> i64 {
    // libc 없이: 같은 순간을 지역시로 포맷해서 차이를 재는 대신,
    // 프로세스 시작 시 한 번만 알아내면 충분하다.
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let out = std::process::Command::new(if cfg!(windows) { "cmd" } else { "date" })
            .args(if cfg!(windows) {
                vec!["/C", "powershell -NoProfile -Command \"[int](Get-Date -UFormat %Z)*3600\""]
            } else {
                vec!["+%z"]
            })
            .output();
        match out {
            Ok(o) => parse_offset(String::from_utf8_lossy(&o.stdout).trim()),
            Err(_) => 0,
        }
    })
}

/// "+0900" 또는 "32400" 을 초로.
fn parse_offset(s: &str) -> i64 {
    if let Ok(secs) = s.parse::<i64>() {
        if s.len() == 5 && (s.starts_with('+') || s.starts_with('-')) {
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let n = secs.abs();
            return sign * ((n / 100) * 3600 + (n % 100) * 60);
        }
        return secs;
    }
    0
}

// ---------- 세션 계산 (OS와 무관, 테스트 가능) ----------

pub struct Tracker {
    idle_break_s: f64,
    session_start: Option<i64>,
    app_start: i64,
    cur_app: String,
    /// 세션이 끊겼다 다시 시작할 때마다 올라간다. 구간을 가르는 열쇠 중 하나다.
    session_id: u64,
}

impl Tracker {
    pub fn new(idle_break_s: f64) -> Self {
        Tracker {
            idle_break_s,
            session_start: None,
            app_start: 0,
            cur_app: String::new(),
            session_id: 0,
        }
    }

    /// 반환: (현재 활동 세션 지속 초, 현재 앱 연속 사용 초)
    pub fn update(&mut self, t: i64, app: &str, idle_s: f64) -> (i64, i64) {
        if idle_s >= self.idle_break_s {
            self.session_start = None;
        } else if self.session_start.is_none() {
            self.session_start = Some(t);
            self.session_id += 1;
        }
        if app != self.cur_app {
            self.cur_app = app.to_string();
            self.app_start = t;
        }
        let session_s = self.session_start.map_or(0, |s| t - s);
        (session_s, t - self.app_start)
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }
}

/// 구간을 가르는 기준. 이 값이 바뀌면 새 줄이 열린다.
#[derive(PartialEq, Clone, Debug)]
pub struct Key {
    pub app: String,
    pub title: Option<String>,
    /// 시간대 통계가 구간에 걸쳐 뭉개지지 않도록 '시'가 바뀌면 자른다.
    pub hour: i64,
    pub session: u64,
}

// ---------- 명령 ----------

fn watch(db: &Connection) {
    let mut tracker = Tracker::new(IDLE_BREAK_S);
    let mut open: Option<(i64, Key, i64)> = None; // (행 id, 구간 열쇠, 누적 입력 시간)
    eprintln!("desklog watch — {} (Ctrl-C로 중지)", db_path().display());
    loop {
        let t = unix_now();
        let idle = platform::idle_seconds();
        let (app, title) = platform::active_window().unwrap_or(("unknown".into(), None));
        let (session_s, app_s) = tracker.update(t, &app, idle);
        let key = Key {
            app: app.clone(),
            title: title.clone(),
            hour: local_hour(t),
            session: tracker.session_id(),
        };
        let active = if idle < ACTIVE_IDLE_S { SAMPLE_S as i64 } else { 0 };

        let r = match &mut open {
            // 같은 구간이 이어지는 중 — 끝 시각만 늘린다
            Some((id, k, acc)) if *k == key => {
                *acc += active;
                db.execute(
                    "UPDATE spans SET end_t=?1, active_s=?2, idle_s=?3, session_s=?4, app_s=?5
                     WHERE id=?6",
                    rusqlite::params![t, *acc, idle, session_s, app_s, *id],
                )
            }
            // 앱·창 제목·시간대·세션 중 하나가 바뀌었다 — 새 줄
            _ => {
                let r = db.execute(
                    "INSERT INTO spans
                       (start_t, end_t, app, title, hour, active_s, idle_s, session_s, app_s)
                     VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![t, app, title, key.hour, active, idle, session_s, app_s],
                );
                if r.is_ok() {
                    open = Some((db.last_insert_rowid(), key, active));
                }
                r
            }
        };
        if let Err(e) = r {
            eprintln!("기록 실패: {e}");
        }
        std::thread::sleep(Duration::from_secs(SAMPLE_S));
    }
}

fn now_json(db: &Connection) -> String {
    let row = db.query_row(
        "SELECT end_t, app, title, idle_s, hour, session_s, app_s, start_t, active_s
         FROM spans ORDER BY end_t DESC LIMIT 1",
        [],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
            ))
        },
    );
    match row {
        Ok((t, app, title, idle, hour, session_s, app_s, start_t, active_s)) => format!(
            "{{\"t\":{},\"age_s\":{},\"app\":{},\"title\":{},\"idle_s\":{:.1},\"hour\":{},\
\"session_s\":{},\"app_s\":{},\"span_s\":{},\"span_active_s\":{}}}",
            t,
            unix_now() - t,
            jstr(&app),
            title.map_or("null".to_string(), |s| jstr(&s)),
            idle,
            hour,
            session_s,
            app_s,
            t - start_t + SAMPLE_S as i64,
            active_s
        ),
        Err(_) => "{\"error\":\"no data — desklog watch 를 먼저 띄워라\"}".to_string(),
    }
}

fn label(db: &Connection, arg: Option<&str>) {
    let worked = match arg {
        Some("yes") => 1,
        Some("no") => 0,
        _ => {
            eprintln!("usage: desklog label yes|no");
            return;
        }
    };
    db.execute(
        "INSERT OR REPLACE INTO labels (t, worked) VALUES (?1, ?2)",
        rusqlite::params![unix_now(), worked],
    )
    .expect("라벨 기록 실패");
}

fn export(db: &Connection) {
    println!("start_t,end_t,len_s,app,title,hour,active_s,idle_s,session_s,app_s");
    let mut stmt = db
        .prepare(
            "SELECT start_t, end_t, app, title, hour, active_s, idle_s, session_s, app_s
             FROM spans ORDER BY start_t",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            let (start_t, end_t) = (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?);
            Ok(format!(
                "{},{},{},{},{},{},{},{:.1},{},{}",
                start_t,
                end_t,
                end_t - start_t + SAMPLE_S as i64,
                csv(&r.get::<_, String>(2)?),
                csv(&r.get::<_, Option<String>>(3)?.unwrap_or_default()),
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, f64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?
            ))
        })
        .unwrap();
    for row in rows.flatten() {
        println!("{row}");
    }
}

/// 구간 길이. 한 틱짜리 구간도 SAMPLE_S 만큼으로 센다.
const LEN: &str = "SUM(end_t - start_t + 5)";

fn top(db: &Connection, days: i64, only: Option<&str>) {
    let since = unix_now() - days * 86400;
    // 앱을 지정하면 모든 집계를 그 앱으로 좁힌다.
    let only_s = only.unwrap_or("");
    let (filter, params): (String, Vec<&dyn rusqlite::ToSql>) = if only.is_some() {
        (" AND app = ?2".into(), vec![&since, &only_s])
    } else {
        (String::new(), vec![&since])
    };
    let p = params.as_slice();

    let (total, typed): (i64, i64) = db
        .query_row(
            &format!(
                "SELECT COALESCE({LEN},0), COALESCE(SUM(active_s),0) FROM spans WHERE end_t >= ?1{filter}"
            ),
            p,
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    if total == 0 {
        match only {
            Some(a) => println!("'{a}' 기록 없음. 앱 이름을 정확히 적었는지 보라."),
            None => println!("기록 없음. desklog watch 를 먼저 띄워라."),
        }
        return;
    }

    let span: (i64, i64) = db
        .query_row(
            &format!("SELECT MIN(start_t), MAX(end_t) FROM spans WHERE end_t >= ?1{filter}"),
            p,
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    println!(
        "\n최근 {}일{} · 기록 {} · 그중 입력 있던 시간 {} · 구간 {}\n",
        days,
        only.map_or(String::new(), |a| format!(" · {a}"),),
        dur(total),
        dur(typed),
        dur(span.1 - span.0)
    );

    println!("앱별  (화면 앞 시간 / 입력 있던 시간)");
    let mut stmt = db
        .prepare(&format!(
            "SELECT app, {LEN} l, SUM(active_s) FROM spans WHERE end_t >= ?1{filter}
             GROUP BY app ORDER BY l DESC LIMIT 12"
        ))
        .unwrap();
    let rows: Vec<(String, i64, i64)> = stmt
        .query_map(p, |r| Ok((r.get(0)?, r.get(1)?, r.get(2).unwrap_or(0))))
        .unwrap()
        .flatten()
        .collect();
    let max = rows.first().map_or(1, |r| r.1);
    for (app, l, ty) in &rows {
        println!(
            "  {:<20} {:>8} / {:>8}  {}",
            trunc(app, 20),
            dur(*l),
            dur(*ty),
            bar(*l, max, 24)
        );
    }

    println!("\n시간대");
    let mut stmt = db
        .prepare(&format!(
            "SELECT hour, {LEN} FROM spans WHERE end_t >= ?1{filter} GROUP BY hour"
        ))
        .unwrap();
    let mut hours = [0i64; 24];
    for (h, l) in stmt
        .query_map(p, |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .unwrap()
        .flatten()
    {
        if (0..24).contains(&h) {
            hours[h as usize] = l;
        }
    }
    let hmax = hours.iter().copied().max().unwrap_or(1).max(1);
    for (h, l) in hours.iter().enumerate() {
        if *l == 0 {
            continue;
        }
        println!("  {h:02}시 {:>8}  {}", dur(*l), bar(*l, hmax, 28));
    }

    if let Some((app, _, _)) = rows.first() {
        println!("\n'{app}' 안에서 본 창 제목");
        let mut stmt = db
            .prepare(&format!(
                "SELECT title, {LEN} l FROM spans
                 WHERE end_t >= ?1 AND app = ?2 AND title IS NOT NULL
                 GROUP BY title ORDER BY l DESC LIMIT 10"
            ))
            .unwrap();
        for (title, l) in stmt
            .query_map(rusqlite::params![since, app], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap()
            .flatten()
        {
            println!("  {:>8}  {}", dur(l), trunc(&title, 60));
        }
    }
    println!();
}

/// 최근 구간을 그대로 본다. 집계하지 않는다 — 잘 돌고 있는지는 원본을 봐야 안다.
fn log(db: &Connection, n: i64) {
    let mut stmt = db
        .prepare(
            "SELECT start_t, end_t, app, title, active_s FROM spans
             ORDER BY end_t DESC LIMIT ?1",
        )
        .unwrap();
    let rows: Vec<(i64, i64, String, Option<String>, i64)> = stmt
        .query_map([n], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .flatten()
        .collect();
    if rows.is_empty() {
        println!("기록 없음. desklog watch 를 먼저 띄워라.");
        return;
    }
    println!(
        "\n{:<17} {:>8} {:>8}  {:<16} {}",
        "구간", "길이", "입력", "앱", "창 제목"
    );
    let mut prev_end: Option<i64> = None;
    for (start_t, end_t, app, title, active_s) in rows.into_iter().rev() {
        // 기록이 끊긴 구간을 눈에 보이게 표시한다. 수집기가 죽었는지 알아야 한다.
        if let Some(p) = prev_end {
            let gap = start_t - p;
            if gap > SAMPLE_S as i64 * 3 {
                println!("{:-^78}", format!(" 기록 끊김 {} ", dur(gap)));
            }
        }
        prev_end = Some(end_t);
        println!(
            "{}~{} {:>8} {:>8}  {:<16} {}",
            hhmmss(start_t),
            hhmmss(end_t),
            dur(end_t - start_t + SAMPLE_S as i64),
            dur(active_s),
            trunc(&app, 16),
            trunc(title.as_deref().unwrap_or("(제목 없음)"), 38)
        );
    }
    println!();
}

fn hhmmss(t: i64) -> String {
    let s = (t + local_utc_offset_seconds()).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

fn live(db: &Connection) {
    eprintln!("Ctrl-C로 중지\n");
    loop {
        print!("\r\x1b[K{}", now_json(db));
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn dur(secs: i64) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h > 0 {
        format!("{h}시간{m:02}분")
    } else {
        format!("{m}분{:02}초", secs % 60)
    }
}

fn bar(v: i64, max: i64, width: usize) -> String {
    let n = ((v as f64 / max.max(1) as f64) * width as f64).round() as usize;
    "\u{2588}".repeat(n.max(1))
}

/// 한글이 섞여 있으니 바이트가 아니라 문자 수로 자른다.
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn csv(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_breaks_after_idle_and_app_time_resets_on_switch() {
        let mut t = Tracker::new(300.0);

        // 활동 시작
        assert_eq!(t.update(1000, "Chrome", 1.0), (0, 0));
        assert_eq!(t.update(1060, "Chrome", 2.0), (60, 60));

        // 앱만 바뀌면 세션은 유지되고 앱 시간만 초기화
        assert_eq!(t.update(1120, "TradingView", 2.0), (120, 0));
        assert_eq!(t.update(1180, "TradingView", 2.0), (180, 60));

        let sid = t.session_id();

        // 5분 넘게 자리 비움 → 세션 끊김
        assert_eq!(t.update(1500, "TradingView", 301.0), (0, 380));

        // 돌아오면 세션이 새로 시작하고, 구간도 갈린다
        assert_eq!(t.update(1560, "TradingView", 1.0), (0, 440));
        assert_eq!(t.session_id(), sid + 1, "세션이 다시 시작되면 번호가 올라가야 한다");
        assert_eq!(t.update(1620, "TradingView", 1.0), (60, 500));
    }

    #[test]
    fn utc_offset_parsing() {
        assert_eq!(parse_offset("+0900"), 32400);
        assert_eq!(parse_offset("-0500"), -18000);
        assert_eq!(parse_offset("+0530"), 19800);
        assert_eq!(parse_offset("32400"), 32400);
        assert_eq!(parse_offset("garbage"), 0);
    }

    #[test]
    fn help_lists_every_command() {
        for cmd in ["watch", "now", "live", "log", "top", "label", "export"] {
            assert!(HELP.contains(cmd), "도움말에 {cmd} 가 빠졌다");
        }
        assert!(HELP.contains("--help") && HELP.contains("--version"));
    }

    #[test]
    fn duration_and_truncation() {
        assert_eq!(dur(0), "0분00초");
        assert_eq!(dur(95), "1분35초");
        assert_eq!(dur(3725), "1시간02분");
        assert_eq!(trunc("짧다", 10), "짧다");
        assert_eq!(trunc("한글여덟글자입니다", 5), "한글여덟…");
    }

    #[test]
    fn json_escapes_titles_with_quotes() {
        assert_eq!(jstr("a\"b"), "\"a\\\"b\"");
        assert_eq!(jstr("한글 - Chrome"), "\"한글 - Chrome\"");
    }
}
