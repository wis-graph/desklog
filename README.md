# desklog

책상 앞에서 무슨 일이 있었는지 기록하는 CLI 수집기.
활성 앱과 창 제목, 마지막 입력 이후 경과 시간을 관측해서 sqlite에 구간 단위로 남긴다.

> Records what you do at your desk — active app, window title, and input idle time —
> as time spans in a local sqlite file. macOS and Windows, no runtime dependencies.

사용자 상태를 관측해서 개입 시점을 고르는 시스템의 관측 부분으로 만들었다.
관측만 하고 판단은 하지 않는다. 판단하는 쪽은 sqlite를 읽어가면 된다.

## 설치

```
brew install wis-graph/tap/desklog
brew services start desklog      # 로그인할 때 자동으로 수집을 시작한다
```

brew 는 미리 빌드해 **Developer ID 로 서명·공증한 universal 바이너리**(arm64 + x86_64)를 받는다.
컴파일하지 않으므로 바로 설치된다. 서명 식별자가 고정돼 있어 판을 올려도 macOS 가 같은 앱으로 본다 —
화면 기록 권한을 한 번 주면 그 뒤로 다시 묻지 않는다.

`brew services`가 launchd에 등록하므로 재부팅해도 알아서 다시 뜬다.
죽으면 다시 띄운다(`keep_alive`).

올릴 때는 한 명령으로 끝난다:

```
desklog update
```

`brew update` → `brew upgrade` → (돌고 있었으면) `brew services restart` 를 순서대로 한다.
판을 올리고 재시작을 빼먹으면 옛 바이너리가 계속 돌기 때문에 한 명령으로 묶었다.
**스스로 판을 확인하러 나가지는 않는다** — 부를 때만 brew 를 시킨다.

```
brew services list | grep desklog     상태 확인
brew services stop desklog            중지
brew services restart desklog         재시작
tail -f $(brew --prefix)/var/log/desklog.log
```

소스에서 직접:

```
cargo build --release
ln -sf "$PWD/target/release/desklog" ~/bin/desklog
```

`~/bin`이 PATH에서 Homebrew보다 앞이면 brew로 설치한 것을 가린다.
둘 다 있으면 `which desklog`로 확인한다.

## 사용

```
desklog watch              상주하며 5초마다 관측한다
desklog now                현재 상태를 JSON 한 줄로 (다른 프로그램이 호출)
desklog live               현재 상태를 1초마다 갱신
desklog log [개수]         구간 원본을 훑어본다 (기본 40)
desklog top [일수] [앱]    앱별 시간·시간대·창 제목 요약 (기본 7일, 앱을 주면 그 앱만)
desklog focus [일수]       한 앱 능동 사용 구간 — 오래 머물며 입력한 시간
desklog note [앱] [설명]   앱 사용 패턴을 적거나 본다
desklog doctor             잘 돌고 있는지, 무엇을 못 읽고 있는지
desklog update             최신판으로 올리고 수집기를 다시 띄운다
desklog label yes|no       직전 개입이 먹혔는지 기록한다 (학습 라벨)
desklog export             학습용 CSV

desklog <읽기명령> --json   top·focus·note·log 을 JSON 으로 (기계·AI 소비용)
```

`label`은 시각과 yes/no 만 `labels` 표에 남긴다. 개입하는 쪽(로봇·AI)이 개입 직후 결과를 적고,
`export`의 구간 기록과 시각으로 맞춰 학습 데이터로 쓴다.

### `now` 출력 필드

로봇·AI 가 가장 자주 부르는 명령이다. watch 가 마지막으로 쓴 구간을 JSON 한 줄로 낸다.

```json
{"t":1790176088,"age_s":1,"app":"Slack","title":"#general","idle_s":11.6,"hour":10,
 "session_s":661,"app_s":120,"span_s":45,"span_active_s":40,"locked":false}
```

| 필드 | 뜻 |
|---|---|
| `t` | 마지막 기록 시각 (유닉스 초) |
| `age_s` | 마지막 기록 이후 지난 초. 10초를 넘으면 watch 가 죽었다 |
| `app` `title` | 최전면 앱, 창 제목 (`title`은 권한이 없으면 `null`) |
| `idle_s` | 마지막 키보드·마우스 입력 이후 초 |
| `hour` | 지역시 '시' |
| `session_s` | 5분 넘는 무입력 없이 이어진 시간. 지금 5분 넘게 입력이 없으면 0 |
| `app_s` | 지금 앱을 연속으로 쓴 시간 |
| `span_s` `span_active_s` | 지금 구간(앱·제목·시·세션·잠금이 같은 동안)의 길이와 그중 입력 있던 시간 |
| `locked` | 화면이 잠겼거나 꺼져 있다 |

`brew services`를 쓰지 않고 직접 띄우려면:

```
nohup desklog watch > /tmp/desklog.log 2>&1 &
```

`log` 출력 예:

```
구간                      길이       입력  앱                창 제목
09:12:05~09:40:10   28분10초   24분35초  Code             main.rs — myproject
09:40:15~09:52:30   12분20초    1분10초  Google Chrome    YouTube
09:52:35~10:00:00    7분30초    6분55초  Slack            #general
```

`길이`(화면 앞에 있던 시간)와 `입력`(실제로 입력이 있던 시간)이 나뉘어 있다.
둘의 차이가 "보고만 있던 시간"이다.

```
desklog top 7 Slack         그 앱만 좁혀서 본다
```

## 몰입 — `focus`

`top`은 입력이 있던 시간을 앱별·시간대별로 나눠 보여준다. 최전면 시간은 둘째 숫자로만 둔다 —
안 자는 기계에서 최전면 시간은 "켜져 있었다"와 같아서, 시간대 막대가 하루 종일 평평하게 나왔다.
`focus`는 거기서 더 나아가 **한 앱에 오래 머물며 입력한 구간**만 센다.

```
최근 7일 · 몰입 기준: 한 앱에 15분 이상, 그중 입력 50% 이상, 60초 이하 딴짓은 무시

날짜     구간            길이    입력  앱         제목
03-02  09:12~10:47   95분12초   86%  Code       main.rs — myproject
03-02  14:05~14:31   26분40초   71%  Code       README.md — myproject
03-03  10:20~11:02   42분05초   64%  Slack      #design

하루별 몰입
  03-02    2시간01분  (구간 2개)
  03-03      42분05초  (구간 1개)
```

Chrome이 최전면 5시간, 입력 7분이라면 — 창을 켜둔 채 자리를 비운 시간이라 `top`에서는 맨 아래로 가고
`focus`에서는 아예 안 나온다.

세 조건이다 — 한 앱에 오래(15분), 그동안 입력이 있었고(50%), 화면이 잠기지 않았다.
시(hour) 경계로 잘린 구간은 다시 붙이고, 60초 이하의 딴 앱 방문(알림 확인 정도)은 끊지 않고,
세션이 다시 시작되면(5분 넘게 입력 없음) 끊는다.

**이건 '집중'을 판별하지 않는다.** desklog 가 정확히 재는 것은 어떤 앱을·얼마나·얼마나 입력하며·언제
썼는지뿐이다. 그게 집중이었는지는 재지 않는다 — 읽기·영상처럼 입력 없는 몰입은 놓치고,
한 창 안의 딴짓은 능동 사용으로 잘못 잡는다. **집중 여부는 "어떤 앱이 집중 작업인가"를 사용자가
정하는 해석의 문제**이고, desklog 는 그 판단의 재료(한 앱 능동 사용 구간)를 줄 뿐이다.
어떤 앱이 작업이고 어떤 앱이 딴짓인지는 desklog 를 읽는 쪽이 정한다 — `top <앱>` 으로 앱별로 좁혀 본다.

## 앱 사용 패턴 — `note`

이 앱의 운영자는 대개 AI(Claude·ChatGPT)다. 집중밀도를 추정하려면 알고리즘이 아니라 인터뷰로 한다:

```
1) desklog top                     이 사람이 주로 쓰는 앱을 본다
2) 사람에게 묻는다                  "Chrome 에서 주로 뭐 해요? 작업? 유튜브?"
3) desklog note "Google Chrome" "리서치 반, 유튜브 반. 입력 적으면 대개 영상 시청"
4) 다음부터 top 이 숫자 옆에 그 패턴을 함께 보여준다
```

```
앱별  (입력 있던 시간 / 최전면 시간)
  Code                  14시간09분 /  31시간02분  ████████████████████████
                         └ 에디터. 코딩 작업. 입력=능동 작업
  Google Chrome           7분35초 /   5시간04분  █
                         └ 리서치 반, 유튜브 반. 입력 적으면 대개 영상 시청
```

`note` 만 치면 적힌 패턴을 모두, `note <앱>` 이면 그 앱만, `note <앱> -` 이면 지운다.
패턴은 `~/.desklog.db` 의 `app_notes` 표에 앱별로 하나씩 저장된다.

desklog 는 숫자(재료)와 패턴(인터뷰 결과)을 나란히 놓아 줄 뿐, **집중밀도는 그걸 읽은 AI 가 매긴다.**
desklog 가 판별하지 않는다는 원칙은 그대로다 — 판별의 재료를 한자리에 모아 둘 뿐이다.

## JSON 출력 — AI 소비용

이 앱의 데이터 소비자는 대개 AI(Claude·ChatGPT)다. 읽기 명령에 `--json` 을 붙이면
사람용 표 대신 기계가 읽을 JSON 을 낸다.

```
desklog top 7 --json      앱별 input_s/frontmost_s(+note), 시간대별 input_s
desklog top 7 Cursor --json   그 앱만
desklog focus 7 --json    능동 사용 구간 배열 (start_t·len_s·app·active_pct·top_title)
desklog note --json       앱별 사용 패턴
desklog log 40 --json     원시 구간 배열
```

정렬·해석 기준은 `input_s`(입력 시간)다 — `top` JSON 의 `basis` 필드가 이를 밝힌다.
`now` 는 원래부터 JSON 이라 `--json` 이 필요 없다. `export`(CSV)는 표계산기용으로 그대로 둔다.

예 — `top --json`:

```json
{
  "days": 7, "basis": "input_s",
  "totals": { "input_s": 51395, "frontmost_s": 130344, "locked_s": 0 },
  "apps": [
    { "app": "Code", "input_s": 50940, "frontmost_s": 111720,
      "note": "에디터. 코딩 작업. 입력=능동 작업" },
    { "app": "Google Chrome", "input_s": 455, "frontmost_s": 18240,
      "note": "리서치 반, 유튜브 반. 입력 적으면 대개 영상 시청" }
  ],
  "hours": [ { "hour": 9, "input_s": 8830 }, ... ]
}
```

숫자(재료)와 note(인터뷰 결과)가 한 응답에 같이 오므로, AI 가 자동화 앱을 빼고
집중밀도를 해석할 수 있다. desklog 는 여전히 판별하지 않는다.

## 직접 조회

sqlite 파일이라 아무 도구로나 읽을 수 있다. `sqlite3 -box ~/.desklog.db` 로 붙는다.

```sql
-- 특정 앱에서 본 창 제목
SELECT title, SUM(end_t-start_t+5) AS 초 FROM spans
WHERE app='Google Chrome' GROUP BY title ORDER BY 초 DESC LIMIT 20;

-- 화면 앞에는 있었지만 입력이 거의 없던 구간 (보고만 있던 시간)
SELECT datetime(start_t,'unixepoch','localtime') AS 시작,
       (end_t-start_t+5)/60 AS 분, app, title
FROM spans WHERE end_t-start_t > 600 AND active_s*10 < end_t-start_t
ORDER BY start_t DESC;

-- 늦은 시각에 무엇을 했나
SELECT hour, app, SUM(end_t-start_t+5) AS 초 FROM spans
WHERE hour BETWEEN 0 AND 5 GROUP BY hour, app ORDER BY hour, 초 DESC;

-- 하루별 총 사용 시간
SELECT date(start_t,'unixepoch','localtime') AS 날짜,
       SUM(end_t-start_t+5)/3600.0 AS 시간
FROM spans GROUP BY 날짜 ORDER BY 날짜 DESC;
```

## 저장 방식

초당 같은 행을 반복 저장하지 않는다. 앱·창 제목·시각(시)·세션 중 하나가 바뀔 때만
새 줄을 열고, 이어지는 동안은 끝 시각만 늘린다. 실측에서 2001행이 6행이 됐다.

| 필드 | 뜻 |
|---|---|
| `start_t` `end_t` | 구간의 시작·끝 (유닉스 초) |
| `app` | 활성 앱 이름 |
| `title` | 활성 창 제목. macOS는 화면 기록 권한이 없으면 `NULL` |
| `hour` | 지역시 기준 '시' |
| `active_s` | 구간 안에서 입력이 있던 시간 |
| `idle_s` | 구간 끝 시점의 유휴 시간 |
| `session_s` `app_s` | 활동 세션 지속, 앱 연속 사용 시간 |
| `locked` | 화면이 잠겼거나 꺼져 있었다. 사람이 볼 수 없는 상태 |

저장 위치는 `~/.desklog.db`.

## 기록하지 않는 것

입력 내용, 화면 이미지, 네트워크 전송. 입력은 마지막 입력 이후 경과 시간만 물어보고
무엇을 눌렀는지 보지 않는다. 저수준 입력 훅을 걸지 않는다.

다만 **창 제목에는 사람 이름이나 문서 제목이 그대로 들어온다.** 메신저 창 제목이
대화 상대 이름인 식이다. 본인 컴퓨터를 본인이 관측하는 것 말고 다른 용도로 쓴다면
창 제목을 그대로 저장할지 패턴만 남길지 먼저 정해야 한다.

## 기록 지우기

```
brew services stop desklog
rm ~/.desklog.db                  # 전부 지운다. 다음 watch 가 빈 파일을 새로 만든다
brew services start desklog
```

일부만 지우려면 sqlite 로 지운다:

```
sqlite3 ~/.desklog.db "DELETE FROM spans WHERE app='Slack'"          # 한 앱
sqlite3 ~/.desklog.db "UPDATE spans SET title=NULL WHERE app='Slack'" # 제목만
```

## OS별로 갈리는 곳

`src/platform.rs` 한 파일이 전부다.

```rust
pub fn active_window() -> Option<(String, Option<String>)>  // 최전면 앱, 창 제목
pub fn frontmost_app() -> Option<String>                     // 창 조회가 실패할 때 앱 이름만
pub fn idle_seconds() -> f64                                 // 마지막 입력 이후 초
pub fn screen_locked() -> bool                               // 화면 잠김·꺼짐
pub fn request_screen_capture()                              // macOS 화면 기록 권한 요청
```

나머지는 OS를 모른다. 실제로 다른 것은 하나 — macOS는 다른 앱의 창 제목을 읽으려면
화면 기록 권한이 필요해서 `None`이 올 수 있다. 윈도우는 권한 없이 항상 준다.

**윈도우 코드는 아직 실기에서 컴파일해본 적 없다.** Homebrew 설치도 macOS만 확인했다.

## 알려진 거친 부분

### macOS · `brew services`로 띄우면 처음엔 창 제목이 비어 있다

CLI 도구의 화면 기록 권한은 실행 파일이 아니라 **띄운 부모 프로세스**에 붙는다.
터미널에서 실행하면 터미널의 권한을 물려받지만, `brew services`(launchd)로 띄우면
물려받을 권한이 없어서 `title`이 `NULL`로 온다.

그래서 `watch`는 시작할 때 권한을 요청한다. **시스템 대화상자가 뜨면 허용하고
`brew services restart desklog`** 하면 그때부터 제목이 읽힌다. 확인은 `desklog doctor` —
최근 24시간에 제목이 찍힌 구간이 있는지로 판정한다.

0.5.2 부터는 서명한 `.app` 번들(`com.wis-graph.desklog`)로 배포한다. macOS 가 번들 식별자로
권한을 묶으므로 판을 올려도 권한을 다시 묻지 않는다. 그 전 판은 판마다 화면 기록 목록에
항목이 새로 생겼다 — 옛 항목은 지우고 한 번만 허용하면 된다.

앱 이름은 권한과 무관하다 — 창 조회가 실패하면 `NSWorkspace`에 최전면 앱을 따로 묻는다.

원시 값을 보려면 `cargo run --release --example probe`.

### 그 밖에

- 앱을 전환하는 짧은 순간에 앱 이름을 못 읽어 `unknown` 구간이 생길 수 있다.
  재시도를 넣지 않았다.

## 개발

```
cargo test
```

판 내는 법은 [RELEASING.md](RELEASING.md). 바꾼 것은 `CHANGELOG.md`의 `## 미출시` 절에 적는다.

## 라이선스

MIT
