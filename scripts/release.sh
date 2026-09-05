#!/usr/bin/env bash
# 판을 올린다. CHANGELOG 의 '미출시' 절이 곧 릴리스 노트다.
#
#   scripts/release.sh patch     0.1.1 → 0.1.2
#   scripts/release.sh minor     0.1.1 → 0.2.0
#   scripts/release.sh major     0.1.1 → 1.0.0
#   scripts/release.sh 0.3.0     직접 지정
#
# 하는 일: 검사 → CHANGELOG 절 확정 → Cargo.toml 판 올림 → 커밋·태그·푸시
#          → 탭 formula 의 url·sha256 갱신 → GitHub 릴리스 생성
set -euo pipefail
cd "$(dirname "$0")/.."

TAP_REPO="git@github.com:wis-graph/homebrew-tap.git"
FORMULA="Formula/desklog.rb"
TARBALL="https://github.com/wis-graph/desklog/archive/refs/tags"

die() { echo "✗ $*" >&2; exit 1; }
say() { echo "▸ $*"; }

# ---- 검사: 되돌리기 어려운 일을 하기 전에 전부 확인한다 ----
[ -n "${1:-}" ] || die "판을 지정해라: patch | minor | major | 1.2.3"
[ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || die "main 브랜치에서만 낸다"
[ -z "$(git status --porcelain)" ] || die "커밋 안 된 변경이 있다"
command -v gh >/dev/null || die "gh 가 없다"
gh auth status >/dev/null 2>&1 || die "gh 로그인이 안 돼 있다"

say "테스트"
cargo test --quiet

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

# '미출시' 절이 비어 있으면 낼 것이 없다
NOTES=$(awk '/^## 미출시/{f=1;next} /^## /{f=0} f' CHANGELOG.md | sed '/^[[:space:]]*$/d')
[ -n "$NOTES" ] || die "CHANGELOG.md 의 '## 미출시' 절이 비어 있다. 뭘 바꿨는지 먼저 적어라"

say "$CUR → $NEW"
echo "$NOTES" | sed 's/^/    /'
printf "진행할까? [y/N] "; read -r ans; [ "$ans" = "y" ] || die "그만둔다"

# ---- 판 올리고 CHANGELOG 절을 확정한다 ----
TODAY=$(date +%Y-%m-%d)
python3 scripts/bump.py "$CUR" "$NEW" "$TODAY"

# 편집이 실제로 먹었는지 확인한다. 0.1.2 를 낼 때 sed 가 조용히 실패해서
# formula 는 0.1.2 인데 바이너리는 0.1.1 을 말하는 일이 있었다.
grep -q "^version = \"$NEW\"$" Cargo.toml || die "Cargo.toml 판 번호가 안 바뀌었다"
grep -q "^## $NEW ($TODAY)$" CHANGELOG.md || die "CHANGELOG 절이 안 만들어졌다"
cargo build --release --quiet
[ "$(./target/release/desklog --version)" = "desklog $NEW" ] || die "바이너리가 $NEW 를 말하지 않는다"

git add -A
git commit -q -m "release: $NEW

$NOTES"
git tag "v$NEW"
git push -q origin main --tags
say "태그 v$NEW 푸시"

# ---- 탭 formula ----
say "sha256 계산"
SHA=$(curl -sL "$TARBALL/v$NEW.tar.gz" | shasum -a 256 | cut -d' ' -f1)
[ ${#SHA} -eq 64 ] || die "sha256 을 못 얻었다"

TAP=$(mktemp -d)
trap 'rm -rf "$TAP"' EXIT
git clone -q "$TAP_REPO" "$TAP"
sed -i '' "s|v[0-9.]*\.tar\.gz|v$NEW.tar.gz|; s|sha256 \".*\"|sha256 \"$SHA\"|" "$TAP/$FORMULA"
git -C "$TAP" commit -qam "chore: desklog $NEW"
git -C "$TAP" push -q origin main
say "탭 갱신"

# ---- 릴리스 ----
gh release create "v$NEW" --title "v$NEW" --notes "$NOTES

## 올리기

\`\`\`
brew update && brew upgrade wis-graph/tap/desklog
brew services restart desklog
\`\`\`"

say "완료 — https://github.com/wis-graph/desklog/releases/tag/v$NEW"
