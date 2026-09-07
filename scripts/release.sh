#!/usr/bin/env bash
# 판을 올린다. CHANGELOG 의 '미출시' 절이 곧 릴리스 노트다.
#
#   scripts/release.sh patch     0.1.1 → 0.1.2
#   scripts/release.sh minor     0.1.1 → 0.2.0
#   scripts/release.sh major     0.1.1 → 1.0.0
#   scripts/release.sh 0.3.0     직접 지정
#
# 하는 일: 검사 → 판 올림·CHANGELOG 확정 → universal 빌드 → Developer ID 서명 → 공증
#          → 커밋·태그·푸시 → GitHub 릴리스(바이너리 첨부) → 탭 formula 갱신
#
# 서명본을 배포하는 이유: macOS TCC(화면 기록 권한)는 서명 식별자로 앱을 알아본다.
# 서명이 없으면 경로로 알아보고, brew 는 판마다 경로가 바뀌어 권한을 매번 다시 묻는다.
set -euo pipefail
cd "$(dirname "$0")/.."

TAP_REPO="git@github.com:wis-graph/homebrew-tap.git"
FORMULA="Formula/desklog.rb"
SIGN_ID="com.wis-graph.desklog"   # 바꾸면 TCC 가 다른 앱으로 본다. 바꾸지 않는다.

die() { echo "✗ $*" >&2; exit 1; }
say() { echo "▸ $*"; }

# Homebrew 의 rust 가 PATH 에서 rustup 을 가린다. 그쪽엔 x86_64 std 가 없다.
# rustup 툴체인의 cargo·rustc 를 경로로 박아 쓴다.
CARGO=$(rustup which cargo) || die "rustup 이 없다"
export RUSTC; RUSTC=$(rustup which rustc)

# ---- 검사: 되돌리기 어려운 일을 하기 전에 전부 확인한다 ----
[ -n "${1:-}" ] || die "판을 지정해라: patch | minor | major | 1.2.3"
[ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || die "main 브랜치에서만 낸다"
[ -z "$(git status --porcelain)" ] || die "커밋 안 된 변경이 있다"
command -v gh >/dev/null || die "gh 가 없다"
gh auth status >/dev/null 2>&1 || die "gh 로그인이 안 돼 있다"
for v in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  [ -n "${!v:-}" ] || die "$v 가 비어 있다 — devAuth 의 signing.env 가 source 되지 않았다"
done
# 주의: pipefail 아래서 `cmd | grep -q` 는 grep 이 먼저 닫아 cmd 가 SIGPIPE 로 죽고 파이프가 실패한다.
# 출력을 변수로 받은 뒤 검사한다.
IDENTS=$(security find-identity -v -p codesigning 2>&1)
grep -q "Developer ID Application" <<<"$IDENTS" || die "키체인에 Developer ID 인증서가 없다"
TARGETS=$(rustup target list --installed)
grep -q x86_64-apple-darwin <<<"$TARGETS" || die "rustup target add x86_64-apple-darwin"

say "테스트"
"$CARGO" test --quiet

CUR=$(grep -m1 '^version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
case "$1" in
  patch|minor|major)
    IFS=. read -r MA MI PA <<<"$CUR"
    case "$1" in
      patch) PA=$((PA+1));;
      minor) MI=$((MI+1)); PA=0;;
      major) MA=$((MA+1)); MI=0; PA=0;;
    esac
    NEW="$MA.$MI.$PA" ;;
  *) NEW="$1" ;;
esac
echo "$NEW" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' || die "판 번호 형식이 아니다: $NEW"
git rev-parse "v$NEW" >/dev/null 2>&1 && die "태그 v$NEW 가 이미 있다"

NOTES=$(awk '/^## 미출시/{f=1;next} /^## /{f=0} f' CHANGELOG.md | sed '/^[[:space:]]*$/d')
[ -n "$NOTES" ] || die "CHANGELOG.md 의 '## 미출시' 절이 비어 있다. 뭘 바꿨는지 먼저 적어라"

say "$CUR → $NEW"
echo "$NOTES" | sed 's/^/    /'
printf "진행할까? [y/N] "; read -r ans; [ "$ans" = "y" ] || die "그만둔다"

# ---- 판 올리고 CHANGELOG 절을 확정한다 ----
TODAY=$(date +%Y-%m-%d)
python3 scripts/bump.py "$CUR" "$NEW" "$TODAY"
grep -q "^version = \"$NEW\"$" Cargo.toml || die "Cargo.toml 판 번호가 안 바뀌었다"
grep -q "^## $NEW ($TODAY)$" CHANGELOG.md || die "CHANGELOG 절이 안 만들어졌다"

# ---- universal 빌드 ----
say "빌드 arm64 + x86_64"
"$CARGO" build --release --quiet
"$CARGO" build --release --quiet --target x86_64-apple-darwin
OUT=target/universal; mkdir -p "$OUT"
lipo -create target/release/desklog target/x86_64-apple-darwin/release/desklog -output "$OUT/desklog"
[ "$("$OUT/desklog" --version)" = "desklog $NEW" ] || die "바이너리가 $NEW 를 말하지 않는다"

# ---- 서명·공증 ----
say "서명 ($SIGN_ID)"
codesign --force --options runtime --timestamp --identifier "$SIGN_ID" \
  --sign "$APPLE_SIGNING_IDENTITY" "$OUT/desklog"
codesign --verify --strict "$OUT/desklog" || die "서명 검증 실패"
SIGINFO=$(codesign -dv "$OUT/desklog" 2>&1)
grep -q "Identifier=$SIGN_ID" <<<"$SIGINFO" || die "식별자가 $SIGN_ID 가 아니다"
grep -q "TeamIdentifier=$APPLE_TEAM_ID" <<<"$SIGINFO" || die "Team ID 가 다르다"

say "공증 (몇 분 걸린다)"
rm -f "$OUT/desklog.zip"
ditto -c -k --keepParent "$OUT/desklog" "$OUT/desklog.zip"
xcrun notarytool submit "$OUT/desklog.zip" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" --wait 2>&1 | tee "$OUT/notarize.log" | grep -E "status:" | tail -1
grep -q "status: Accepted" "$OUT/notarize.log" || die "공증이 거절됐다 — $OUT/notarize.log"

ASSET="desklog-$NEW-macos-universal.tar.gz"
tar -czf "$OUT/$ASSET" -C "$OUT" desklog
SHA=$(shasum -a 256 "$OUT/$ASSET" | cut -d' ' -f1)

# ---- 커밋·태그·푸시·릴리스 ----
git add -A
git commit -q -m "release: $NEW

$NOTES"
git tag "v$NEW"
git push -q origin main --tags
say "태그 v$NEW 푸시"

gh release create "v$NEW" "$OUT/$ASSET" --title "v$NEW" --notes "$NOTES

## 올리기

\`\`\`
brew update && brew upgrade wis-graph/tap/desklog
brew services restart desklog
\`\`\`

macOS universal (arm64 + x86_64), Developer ID 서명·공증. sha256 \`$SHA\`"
say "릴리스 생성 (바이너리 첨부)"

# ---- 탭 formula: 통째로 다시 쓴다. 줄 하나씩 고치다 어긋난 적이 있다 ----
TAP=$(mktemp -d)
trap 'rm -rf "$TAP"' EXIT
git clone -q "$TAP_REPO" "$TAP"
cat > "$TAP/$FORMULA" <<EOF
class Desklog < Formula
  desc "Records what you do at your desk as time spans in a local sqlite file"
  homepage "https://github.com/wis-graph/desklog"
  url "https://github.com/wis-graph/desklog/releases/download/v$NEW/$ASSET"
  sha256 "$SHA"
  version "$NEW"
  license "MIT"

  # 미리 빌드해 Developer ID 로 서명·공증한 universal 바이너리를 받는다.
  # 소스 빌드로 바꾸면 서명이 사라지고 화면 기록 권한을 판마다 다시 묻게 된다.
  depends_on :macos

  def install
    bin.install "desklog"
  end

  service do
    run [opt_bin/"desklog", "watch"]
    keep_alive true
    log_path var/"log/desklog.log"
    error_log_path var/"log/desklog.log"
  end

  test do
    assert_match "desklog", shell_output("#{bin}/desklog --version")
    assert_match "watch", shell_output("#{bin}/desklog --help")
  end
end
EOF
git -C "$TAP" commit -qam "chore: desklog $NEW"
git -C "$TAP" push -q origin main
say "탭 갱신"

say "완료 — https://github.com/wis-graph/desklog/releases/tag/v$NEW"
