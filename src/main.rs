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
  doctor             잘 돌고 있는지, 무엇을 못 읽고 있는지 진단한다
  focus [일수]       몰입 구간 — 한 앱에 오래, 입력을 하면서 머문 시간 (기본 7일)

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
        "doctor" => doctor(&db),
        "focus" => focus(&db, args.get(1).and_then(|s| s.parse().ok()).unwrap_or(7)),
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
            app_s      INTEGER NOT NULL,
            locked     INTEGER NOT NULL DEFAULT 0  -- 화면이 잠겼거나 꺼져 있었다
         );
         CREATE INDEX IF NOT EXISTS idx_spans_start ON spans(start_t);
         CREATE TABLE IF NOT EXISTS labels (
            t      INTEGER PRIMARY KEY,
            worked INTEGER NOT NULL
         );",
    )
    .expect("스키마 생성 실패");
    // 0.1.x 로 만든 DB 에는 locked 열이 없다. 있는지 보고 없으면 더한다.
    let has_locked: bool = db
        .prepare("PRAGMA table_info(spans)")
        .and_then(|mut st| {
            let names: Vec<String> = st.query_map([], |r| r.get::<_, String>(1))?.flatten().collect();
            Ok(names.iter().any(|n| n == "locked"))
        })
        .unwrap_or(true);
    if !has_locked {
        db.execute_batch("ALTER TABLE spans ADD COLUMN locked INTEGER NOT NULL DEFAULT 0")
            .expect("locked 열 추가 실패");
    }
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
    /// 잠금·해제 순간에 구간이 갈리게 한다.
    pub locked: bool,
}

// ---------- 명령 ----------

fn watch(db: &Connection) {
    let mut tracker = Tracker::new(IDLE_BREAK_S);
    let mut open: Option<(i64, Key, i64)> = None; // (행 id, 구간 열쇠, 누적 입력 시간)
    eprintln!("desklog watch — {} (Ctrl-C로 중지)", db_path().display());
    if !platform::screen_capture_allowed() {
        eprintln!("화면 기록 권한이 없어 창 제목을 못 읽는다. 요청한다 — 허용하면 다음 실행부터 적용된다.");
        platform::request_screen_capture();
    }
    loop {
        let t = unix_now();
        let idle = platform::idle_seconds();
        let (app, title) = platform::active_window().unwrap_or(("unknown".into(), None));
        let locked = platform::screen_locked();
        let (session_s, app_s) = tracker.update(t, &app, idle);
        let key = Key {
            app: app.clone(),
            title: title.clone(),
            hour: local_hour(t),
            session: tracker.session_id(),
            locked,
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
            // 앱·창 제목·시간대·세션·잠금 중 하나가 바뀌었다 — 새 줄
            _ => {
                let r = db.execute(
                    "INSERT INTO spans
                       (start_t, end_t, app, title, hour, active_s, idle_s, session_s, app_s, locked)
                     VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![t, app, title, key.hour, active, idle, session_s, app_s, locked],
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
        "SELECT end_t, app, title, idle_s, hour, session_s, app_s, start_t, active_s, locked
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
                r.get::<_, bool>(9)?,
            ))
        },
    );
    match row {
        Ok((t, app, title, idle, hour, session_s, app_s, start_t, active_s, locked)) => format!(
            "{{\"t\":{},\"age_s\":{},\"app\":{},\"title\":{},\"idle_s\":{:.1},\"hour\":{},\
\"session_s\":{},\"app_s\":{},\"span_s\":{},\"span_active_s\":{},\"locked\":{}}}",
            t,
            unix_now() - t,
            jstr(&app),
            title.map_or("null".to_string(), |s| jstr(&s)),
            idle,
            hour,
            session_s,
            app_s,
            t - start_t + SAMPLE_S as i64,
            active_s,
            locked
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
    println!("start_t,end_t,len_s,app,title,hour,active_s,idle_s,session_s,app_s,locked");
    let mut stmt = db
        .prepare(
            "SELECT start_t, end_t, app, title, hour, active_s, idle_s, session_s, app_s, locked
             FROM spans ORDER BY start_t",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            let (start_t, end_t) = (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?);
            Ok(format!(
                "{},{},{},{},{},{},{},{:.1},{},{},{}",
                start_t,
                end_t,
                end_t - start_t + SAMPLE_S as i64,
                csv(&r.get::<_, String>(2)?),
                csv(&r.get::<_, Option<String>>(3)?.unwrap_or_default()),
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, f64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?
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

    // 화면이 잠긴 구간은 "화면 앞 시간"이 아니다. 빼되, 뺀 양은 머리줄에 보여준다.
    let (total, typed, locked_s): (i64, i64, i64) = db
        .query_row(
            &format!(
                "SELECT COALESCE(SUM(CASE WHEN locked=0 THEN end_t-start_t+5 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN locked=0 THEN active_s ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN locked=1 THEN end_t-start_t+5 ELSE 0 END),0)
                 FROM spans WHERE end_t >= ?1{filter}"
            ),
            p,
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or((0, 0, 0));
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
        "\n최근 {}일{} · 최전면 {} · 그중 입력 {} · 화면 잠김 {} · 구간 {}\n",
        days,
        only.map_or(String::new(), |a| format!(" · {a}"),),
        dur(total),
        dur(typed),
        dur(locked_s),
        dur(span.1 - span.0)
    );

    println!("앱별  (최전면 시간 / 입력 있던 시간)");
    let mut stmt = db
        .prepare(&format!(
            "SELECT app, {LEN} l, SUM(active_s) FROM spans WHERE end_t >= ?1 AND locked=0{filter}
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
            "SELECT hour, {LEN} FROM spans WHERE end_t >= ?1 AND locked=0{filter} GROUP BY hour"
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
                 WHERE end_t >= ?1 AND app = ?2 AND title IS NOT NULL AND locked=0
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
            "SELECT start_t, end_t, app, title, active_s, locked FROM spans
             ORDER BY end_t DESC LIMIT ?1",
        )
        .unwrap();
    let rows: Vec<(i64, i64, String, Option<String>, i64, bool)> = stmt
        .query_map([n], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
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
    for (start_t, end_t, app, title, active_s, locked) in rows.into_iter().rev() {
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
            if locked {
                "[화면 잠김]".to_string()
            } else {
                trunc(title.as_deref().unwrap_or("(제목 없음)"), 38)
            }
        );
    }
    println!();
}

// ---------- 몰입 구간 ----------
//
// 몰입은 "같은 화면이 오래 떠 있었다"가 아니다. 사람이 거기서 무언가를 하고 있어야 한다.
// 그래서 세 조건이다 — 한 앱에 오래, 그 동안 입력이 있었고, 화면이 잠기지 않았다.

/// 이보다 짧으면 몰입으로 치지 않는다.
const FOCUS_MIN_S: i64 = 15 * 60;
/// 구간 길이 대비 입력이 있던 시간이 이보다 적으면 보고만 있던 것이다.
const FOCUS_MIN_ACTIVE_PCT: i64 = 50;
/// 이보다 짧은 딴 앱 방문은 몰입을 끊지 않는다. 알림 확인 정도.
const FOCUS_GAP_TOLERANCE_S: i64 = 60;

#[derive(Clone, Debug)]
pub struct Row {
    pub start_t: i64,
    pub end_t: i64,
    pub app: String,
    pub title: Option<String>,
    pub active_s: i64,
    pub session_s: i64,
    pub locked: bool,
}

#[derive(Debug, PartialEq)]
pub struct Block {
    pub start_t: i64,
    pub end_t: i64,
    pub app: String,
    pub active_s: i64,
    /// 제목별 머문 시간. 가장 오래 본 제목을 대표로 쓴다.
    pub titles: Vec<(String, i64)>,
}

impl Block {
    pub fn len_s(&self) -> i64 {
        self.end_t - self.start_t + SAMPLE_S as i64
    }
    pub fn active_pct(&self) -> i64 {
        self.active_s * 100 / self.len_s().max(1)
    }
    pub fn top_title(&self) -> Option<&str> {
        self.titles.iter().max_by_key(|(_, s)| *s).map(|(t, _)| t.as_str())
    }
}

/// 구간들을 앱 단위로 이어 붙여 몰입 후보를 만든다. 시(hour)로 잘린 것은 다시 붙고,
/// 제목이 바뀐 것도 같은 앱이면 붙는다. 끊는 것은 넷 — 다른 앱(허용치 초과), 잠금, 세션 재시작, 기록 공백.
pub fn build_blocks(rows: &[Row]) -> Vec<Block> {
    // 1) 같은 앱이 이어지는 만큼 붙인다
    let mut runs: Vec<(Block, bool)> = Vec::new(); // (블록, 잠김)
    let mut prev: Option<&Row> = None;
    for r in rows {
        let len = r.end_t - r.start_t + SAMPLE_S as i64;
        let breaks = match prev {
            None => true,
            Some(p) => {
                r.locked != p.locked
                    || r.app != p.app
                    || r.start_t - p.end_t > SAMPLE_S as i64 * 3 // 기록 공백
                    || r.session_s < p.session_s // 세션이 다시 시작됐다 = 5분 넘게 자리 비움
            }
        };
        if breaks {
            runs.push((
                Block {
                    start_t: r.start_t,
                    end_t: r.end_t,
                    app: r.app.clone(),
                    active_s: r.active_s,
                    titles: r.title.iter().map(|t| (t.clone(), len)).collect(),
                },
                r.locked,
            ));
        } else {
            let (b, _) = runs.last_mut().unwrap();
            b.end_t = r.end_t;
            b.active_s += r.active_s;
            if let Some(t) = &r.title {
                match b.titles.iter_mut().find(|(x, _)| x == t) {
                    Some((_, s)) => *s += len,
                    None => b.titles.push((t.clone(), len)),
                }
            }
        }
        prev = Some(r);
    }

    // 2) 짧은 딴 앱 방문이 같은 앱 사이에 끼어 있으면 삼킨다. 안정될 때까지.
    loop {
        let mut merged = false;
        let mut out: Vec<(Block, bool)> = Vec::new();
        let mut i = 0;
        while i < runs.len() {
            let is_short_gap = i + 1 < runs.len()
                && !out.is_empty()
                && !runs[i].1
                && runs[i].0.len_s() <= FOCUS_GAP_TOLERANCE_S
                && out.last().unwrap().0.app == runs[i + 1].0.app
                && !out.last().unwrap().1
                && !runs[i + 1].1
                && runs[i + 1].0.start_t - out.last().unwrap().0.end_t <= FOCUS_GAP_TOLERANCE_S + SAMPLE_S as i64 * 2;
            if is_short_gap {
                let gap = &runs[i].0;
                let next = &runs[i + 1].0;
                let (last, _) = out.last_mut().unwrap();
                last.end_t = next.end_t;
                last.active_s += gap.active_s + next.active_s;
                for (t, s) in &next.titles {
                    match last.titles.iter_mut().find(|(x, _)| x == t) {
                        Some((_, acc)) => *acc += s,
                        None => last.titles.push((t.clone(), *s)),
                    }
                }
                i += 2;
                merged = true;
            } else {
                out.push((runs[i].0.clone(), runs[i].1));
                i += 1;
            }
        }
        runs = out;
        if !merged {
            break;
        }
    }

    runs.into_iter().filter(|(_, locked)| !locked).map(|(b, _)| b).collect()
}

impl Clone for Block {
    fn clone(&self) -> Self {
        Block {
            start_t: self.start_t,
            end_t: self.end_t,
            app: self.app.clone(),
            active_s: self.active_s,
            titles: self.titles.clone(),
        }
    }
}

pub fn is_focus(b: &Block) -> bool {
    b.len_s() >= FOCUS_MIN_S && b.active_pct() >= FOCUS_MIN_ACTIVE_PCT
}

fn focus(db: &Connection, days: i64) {
    let since = unix_now() - days * 86400;
    let mut stmt = db
        .prepare(
            "SELECT start_t, end_t, app, title, active_s, session_s, locked FROM spans
             WHERE end_t >= ?1 ORDER BY start_t",
        )
        .unwrap();
    let rows: Vec<Row> = stmt
        .query_map([since], |r| {
            Ok(Row {
                start_t: r.get(0)?,
                end_t: r.get(1)?,
                app: r.get(2)?,
                title: r.get(3)?,
                active_s: r.get(4)?,
                session_s: r.get(5)?,
                locked: r.get(6)?,
            })
        })
        .unwrap()
        .flatten()
        .collect();
    if rows.is_empty() {
        println!("기록 없음. desklog watch 를 먼저 띄워라.");
        return;
    }

    let blocks: Vec<Block> = build_blocks(&rows).into_iter().filter(is_focus).collect();
    println!(
        "\n최근 {days}일 · 몰입 기준: 한 앱에 {}분 이상, 그중 입력 {}% 이상, {}초 이하 딴짓은 무시\n",
        FOCUS_MIN_S / 60,
        FOCUS_MIN_ACTIVE_PCT,
        FOCUS_GAP_TOLERANCE_S
    );
    if blocks.is_empty() {
        println!("몰입 구간 없음.\n");
        return;
    }

    println!("{:<6} {:<12} {:>8} {:>5}  {:<16} {}", "날짜", "구간", "길이", "입력", "앱", "제목");
    for b in &blocks {
        println!(
            "{:<6} {}~{} {:>8} {:>4}%  {:<16} {}",
            mmdd(b.start_t),
            hhmm(b.start_t),
            hhmm(b.end_t),
            dur(b.len_s()),
            b.active_pct(),
            trunc(&b.app, 16),
            trunc(b.top_title().unwrap_or("-"), 36)
        );
    }

    let mut by_day: Vec<(String, i64, i64)> = Vec::new();
    let mut by_app: Vec<(String, i64)> = Vec::new();
    for b in &blocks {
        let d = mmdd(b.start_t);
        match by_day.iter_mut().find(|(x, _, _)| *x == d) {
            Some((_, s, n)) => {
                *s += b.len_s();
                *n += 1;
            }
            None => by_day.push((d, b.len_s(), 1)),
        }
        match by_app.iter_mut().find(|(x, _)| *x == b.app) {
            Some((_, s)) => *s += b.len_s(),
            None => by_app.push((b.app.clone(), b.len_s())),
        }
    }
    by_app.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n하루별 몰입");
    for (d, s, n) in &by_day {
        println!("  {d}  {:>8}  (구간 {n}개)", dur(*s));
    }
    println!("\n앱별 몰입");
    let max = by_app.first().map_or(1, |x| x.1);
    for (a, s) in &by_app {
        println!("  {:<16} {:>8}  {}", trunc(a, 16), dur(*s), bar(*s, max, 24));
    }
    println!();
}

fn mmdd(t: i64) -> String {
    let days = (t + local_utc_offset_seconds()).div_euclid(86400);
    // 1970-01-01 부터의 날수 → 월/일. 윤년 계산은 civil_from_days 알고리즘.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{m:02}-{d:02}")
}

fn hhmm(t: i64) -> String {
    let s = (t + local_utc_offset_seconds()).rem_euclid(86400);
    format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// 잘 돌고 있는지, 무엇을 못 읽고 있는지. 문제가 조용히 지나가지 않게 한다.
fn doctor(db: &Connection) {
    let now = unix_now();
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());
    println!("\n실행 파일    {exe}");
    println!("DB           {}", db_path().display());

    let (rows, last): (i64, Option<i64>) = db
        .query_row("SELECT COUNT(*), MAX(end_t) FROM spans", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap_or((0, None));
    match last {
        Some(t) if now - t <= SAMPLE_S as i64 * 3 => println!("수집기      돌고 있다 ({}초 전 기록, 구간 {rows}개)", now - t),
        Some(t) => println!("수집기      ✗ 멈춰 있다 — 마지막 기록 {} 전. brew services restart desklog", dur(now - t)),
        None => println!("수집기      ✗ 기록이 하나도 없다 — desklog watch 를 띄워라"),
    }

    if platform::screen_capture_allowed() {
        println!("화면 기록    허용 — 창 제목을 읽는다");
    } else {
        println!("화면 기록    ✗ 없음 — 창 제목을 못 읽는다 (앱 이름은 읽는다)");
        println!("             시스템 설정 → 개인정보 보호 및 보안 → 화면 기록 → + → 위 실행 파일 추가");
        println!("             brew 로 올릴 때마다 경로가 바뀌어 다시 추가해야 한다");
    }

    // 최근 한 시간, 잠기지 않은 구간 중 제목 없는 비율. 권한 여부와 실제가 맞는지 본다.
    let (all, none): (i64, i64) = db
        .query_row(
            "SELECT COALESCE(SUM(end_t-start_t+5),0),
                    COALESCE(SUM(CASE WHEN title IS NULL THEN end_t-start_t+5 ELSE 0 END),0)
             FROM spans WHERE end_t >= ?1 AND locked=0",
            [now - 3600],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    if all > 0 {
        let pct = none * 100 / all;
        println!("최근 1시간   제목 없는 비율 {pct}%");
        // 이 명령은 터미널 권한으로 돌고, 수집기는 launchd 권한으로 돈다. 둘이 다를 수 있다.
        if pct >= 90 && platform::screen_capture_allowed() {
            println!("             ✗ 여기서는 권한이 있는데 기록에는 제목이 없다 — 수집기가 다른 권한 맥락(launchd)에서 돌고 있다");
            println!("             brew services 로 띄운 실행 파일에 따로 권한을 줘야 한다: $(brew --prefix)/opt/desklog/bin/desklog");
        }
    }

    let (app, title) = platform::active_window().unwrap_or(("(못 읽음)".into(), None));
    println!(
        "지금         앱={app}  제목={}  유휴={:.0}초  잠김={}",
        title.as_deref().unwrap_or("(없음)"),
        platform::idle_seconds(),
        platform::screen_locked()
    );
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

    fn row(start: i64, end: i64, app: &str, active: i64, session: i64, locked: bool) -> Row {
        Row {
            start_t: start,
            end_t: end,
            app: app.into(),
            title: None,
            active_s: active,
            session_s: session,
            locked,
        }
    }

    #[test]
    fn focus_needs_input_not_just_a_window() {
        // 같은 앱 60분, 입력 0 → 몰입 아님 (자리를 비웠거나 보고만 있었다)
        let staring = build_blocks(&[row(0, 3595, "Chrome", 0, 3600, false)]);
        assert_eq!(staring.len(), 1);
        assert!(!is_focus(&staring[0]), "입력 없는 60분은 몰입이 아니다");

        // 같은 앱 30분, 입력 25분 → 몰입
        let working = build_blocks(&[row(0, 1795, "Code", 1500, 1800, false)]);
        assert!(is_focus(&working[0]));
    }

    #[test]
    fn focus_joins_hour_splits_and_swallows_short_detours() {
        let rows = [
            row(0, 595, "Code", 600, 600, false),      // 10분
            row(600, 1195, "Code", 600, 1200, false),  // 시(hour) 경계로 잘린 다음 10분
            row(1200, 1220, "카카오톡", 25, 1225, false), // 25초 알림 확인
            row(1225, 1795, "Code", 570, 1800, false), // 다시 10분
        ];
        let b = build_blocks(&rows);
        assert_eq!(b.len(), 1, "시 경계와 짧은 딴짓을 넘어 하나로 붙어야 한다: {b:?}");
        assert_eq!(b[0].len_s(), 1800);
        assert!(is_focus(&b[0]));
    }

    #[test]
    fn focus_breaks_on_session_restart_and_lock() {
        let rows = [
            row(0, 1795, "Code", 1800, 1800, false),
            row(1800, 2395, "Code", 0, 0, false), // session_s 가 0 으로 돌아왔다 = 자리 비웠다 돌아옴
            row(2400, 2995, "Code", 600, 600, true), // 잠김
        ];
        let b = build_blocks(&rows);
        assert_eq!(b.len(), 2, "세션 재시작에서 끊기고 잠긴 구간은 빠져야 한다: {b:?}");
        assert_eq!(b[0].len_s(), 1800);
    }

    #[test]
    fn help_lists_every_command() {
        for cmd in ["watch", "now", "live", "log", "top", "label", "export", "doctor", "focus"] {
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
