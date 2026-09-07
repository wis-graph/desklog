# 판 내는 법

릴리스 노트를 따로 쓰지 않는다. **`CHANGELOG.md`의 `## 미출시` 절이 곧 릴리스 노트다.**

## 평소

무언가 바꾸면 `CHANGELOG.md`의 `## 미출시` 아래에 한 줄 적는다.
무엇을 바꿨는지가 아니라 **왜 바꿨는지**가 남아야 쓸모가 있다.

```markdown
## 미출시

- `top`에 앱 지정을 더했다. 전에는 1위 앱의 창 제목만 볼 수 있었다.
```

## 낼 때

```
scripts/release.sh patch     0.1.1 → 0.1.2
scripts/release.sh minor     0.1.1 → 0.2.0
scripts/release.sh major     0.1.1 → 1.0.0
scripts/release.sh 0.3.0     직접 지정
```

스크립트가 순서대로 한다.

1. **검사** — main 브랜치, 커밋 안 된 변경 없음, `gh` 로그인, 서명 환경변수와 인증서,
   x86_64 타깃, `cargo test`, 태그 중복, `미출시` 절이 비어 있지 않음
2. 바뀔 내용을 보여주고 **한 번 묻는다**
3. `Cargo.toml` 판 번호를 올리고 `## 미출시` → `## 0.3.0 (날짜)` 로 확정한다.
   편집이 먹었는지 확인한다
4. **arm64 + x86_64 를 빌드해 universal 바이너리로 합친다**. 판 번호를 말하는지 확인한다
5. **Developer ID 로 서명한다** — 식별자 `com.wis-graph.desklog`, hardened runtime
6. **공증한다** (`notarytool --wait`, 몇 분). 거절되면 멈춘다
7. 커밋 · 태그 · 푸시
8. **GitHub 릴리스를 만들고 바이너리 tarball 을 첨부한다**
9. **탭 formula 를 통째로 다시 써서** 그 tarball 을 가리키게 하고 푸시한다

검사를 먼저 다 하고 나서 되돌리기 어려운 일(푸시·태그·릴리스)을 시작한다.

## 왜 서명본을 배포하는가

macOS 의 화면 기록 권한(TCC)은 **서명 식별자 + Team ID** 로 앱을 알아본다.
서명이 없으면 경로와 코드 해시로 알아보는데, brew 는 판마다 `Cellar/desklog/<판>/` 경로가
바뀌어 권한을 매번 다시 묻는다. 서명하면 판이 바뀌어도 같은 앱이다.

그래서 formula 는 소스를 빌드하지 않고 서명된 바이너리를 받는다. 소스 빌드로 되돌리면
사용자 기기에서 서명 없이 빌드되어 권한 문제가 돌아온다.

## 필요한 것

- Developer ID Application 인증서가 키체인에 있어야 한다
- `APPLE_SIGNING_IDENTITY` `APPLE_ID` `APPLE_PASSWORD` `APPLE_TEAM_ID` —
  devAuth 의 `env/signing.env` 를 `~/.zprofile` 이 source 한다. 값은 어디에도 적지 않는다
- `rustup target add x86_64-apple-darwin`

## 걸리는 것

**Homebrew 의 `rust` 가 rustup 을 가린다.** 예전 formula 가 `rust` 를 빌드 의존성으로 깔았고,
`/opt/homebrew/bin/cargo` 가 PATH 에서 `~/.cargo/bin` 보다 앞이다. 그쪽엔 x86_64 표준
라이브러리가 없어 크로스 빌드가 `can't find crate for core` 로 죽는다. `rustup run` 도 cargo 가
`rustc` 를 이름으로 불러 못 피한다. 스크립트는 `rustup which cargo` / `rustup which rustc`
경로를 박아 쓴다. 더 이상 필요 없으면 `brew uninstall rust`.

## 사용자에게 닿는 경로

| 경로 | 사용자가 하는 일 |
|---|---|
| Homebrew | `brew update && brew upgrade wis-graph/tap/desklog` |
| GitHub Releases | 저장소 Watch → Releases only |
| `CHANGELOG.md` | 저장소에서 바로 읽는다 |

**판 확인 기능은 넣지 않는다.** README에 네트워크 전송을 하지 않는다고 적어놨고,
판 번호를 확인하는 것도 외부 요청이다. 감시 도구가 몰래 서버에 연결하면 그 약속이 깨진다.

## 탭

formula는 별도 저장소에 있다 — https://github.com/wis-graph/homebrew-tap

스크립트가 낼 때마다 새로 복제해서 고치고 지운다. 손으로 관리하는 사본을 두지 않는다.
