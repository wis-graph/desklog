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

`brew services`가 launchd에 등록하므로 재부팅해도 알아서 다시 뜬다.
죽으면 다시 띄운다(`keep_alive`).

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
desklog focus [일수]       몰입 구간 — 한 앱에 오래, 입력을 하면서 머문 시간
desklog doctor             잘 돌고 있는지, 무엇을 못 읽고 있는지
desklog label yes|no       라벨 기록
desklog export             학습용 CSV
```

`brew services`를 쓰지 않고 직접 띄우려면:

```
nohup desklog watch > /tmp/desklog.log 2>&1 &
```

`log` 출력 예:

```
구간                      길이       입력  앱                창 제목
17:36:50~17:59:58   23분13초    0분00초  Ghostty          430course-material
18:00:03~18:59:56   59분58초    0분00초  Ghostty          430course-material
20:00:04~20:23:52   23분53초   13분10초  Ghostty          430course-material
```

`길이`(화면 앞에 있던 시간)와 `입력`(실제로 입력이 있던 시간)이 나뉘어 있다.
둘의 차이가 "보고만 있던 시간"이다.

```
desklog top 7 카카오톡      그 앱만 좁혀서 본다
```

## 몰입 — `focus`

`top`은 앱이 최전면에 있던 시간을 센다. 자리를 비운 26시간도 센다.
`focus`는 **사람이 거기서 무언가를 하고 있던 시간**만 센다.

```
최근 7일 · 몰입 기준: 한 앱에 15분 이상, 그중 입력 50% 이상, 60초 이하 딴짓은 무시

날짜     구간            길이    입력  앱         제목
09-05  20:23~20:43   19분37초   99%  카카오톡    김보라
09-05  21:29~22:21   52분31초   98%  카카오톡    -
09-05  23:42~00:39   56분54초   97%  카카오톡    -

하루별 몰입
  09-05    3시간27분  (구간 5개)
```

같은 이틀치 데이터에서 `top`은 Chrome 27시간을 1위로 놓고, `focus`는 Chrome을 아예 세지 않는다.
Chrome 27시간은 입력 7분이었다 — 창을 켜둔 채 자리를 비운 시간이다.

세 조건이다 — 한 앱에 오래(15분), 그동안 입력이 있었고(50%), 화면이 잠기지 않았다.
같은 화면이 떠 있는 것만으로는 몰입이 아니다. 시(hour) 경계로 잘린 구간은 다시 붙이고,
60초 이하의 딴 앱 방문(알림 확인 정도)은 끊지 않는다. 세션이 다시 시작되면(5분 넘게 입력 없음) 끊는다.

이건 판단이 아니라 서술이다. "47분간 한 앱에서 입력하며 머물렀다"까지가 desklog 몫이고,
그게 좋은 일인지 개입할 일인지는 읽는 쪽이 정한다.

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

## OS별로 갈리는 곳

`src/platform.rs` 한 파일, 함수 두 개가 전부다.

```rust
pub fn active_window() -> Option<(String, Option<String>)>
pub fn idle_seconds() -> f64
```

나머지는 OS를 모른다. 실제로 다른 것은 하나 — macOS는 다른 앱의 창 제목을 읽으려면
화면 기록 권한이 필요해서 `None`이 올 수 있다. 윈도우는 권한 없이 항상 준다.

**윈도우 코드는 아직 실기에서 컴파일해본 적 없다.** Homebrew 설치도 macOS만 확인했다.

## 알려진 거친 부분

### macOS · `brew services`로 띄우면 창 제목이 비어 있다

CLI 도구의 화면 기록 권한은 실행 파일이 아니라 **띄운 부모 프로세스**에 붙는다.
터미널에서 실행하면 터미널의 권한을 물려받아 창 제목이 읽히지만,
`brew services`(launchd)로 띄우면 물려받을 권한이 없어서 `title`이 계속 `NULL`이다.

앱 이름은 영향이 없다 — 창 조회가 실패하면 `NSWorkspace`에 최전면 앱을 따로 묻고,
그쪽은 권한이 필요 없다.

창 제목이 필요하면 둘 중 하나를 한다.

- 시스템 설정 → 개인정보 보호 및 보안 → 화면 기록에서 `$(brew --prefix)/opt/desklog/bin/desklog`를
  직접 추가한다
- `brew services`를 쓰지 않고 터미널에서 `nohup desklog watch &` 로 띄운다
  (재부팅하면 다시 띄워야 한다)

무엇이 읽히는지 확인하려면 `desklog doctor`. 터미널에서는 권한이 있는데 기록에 제목이
없으면, 수집기가 다른 권한 맥락(launchd)에서 돌고 있다고 짚어준다.
`watch`는 시작할 때 권한이 없으면 요청하므로, `brew services start` 뒤에 시스템 대화상자가
뜨면 허용하고 `brew services restart desklog` 하면 된다.

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
