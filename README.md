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
desklog top [일수]         앱별 시간·시간대·창 제목 요약 (기본 7)
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

- 앱을 전환하는 순간 `active_window()`가 `None`을 반환해서 `app`이 `unknown`인
  짧은 구간이 생길 때가 있다. 재시도를 넣지 않았다.
- macOS에서 창 제목을 읽으려면 화면 기록 권한이 필요하다. 권한은 실행 파일마다
  따로 물어보므로, 소스 빌드본에 권한을 줬더라도 brew 설치본은 다시 물어볼 수 있다.

## 개발

```
cargo test
```

## 라이선스

MIT
