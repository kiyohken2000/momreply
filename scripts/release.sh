#!/bin/sh
# GitHub Releases に公開する。
#
# 前提として ./scripts/build.sh が済んでいること。署名・公証・更新用の
# 署名がすべて付いた状態で走らせる。
#
# # latest.json
#
# 自動更新はこのファイルを見る。**手で書くと必ず間違える**ので、
# ビルド結果から組み立てる。署名（.sig の中身）を入れ間違えると、
# 更新は「署名が違う」として黙って拒否される。
#
# 置き場所は tauri.conf.json の endpoints と一致していること。
#   https://github.com/<user>/<repo>/releases/latest/download/latest.json
#
# 使い方:
#   ./scripts/release.sh v0.1.0
set -eu

cd "$(dirname "$0")/.."

TAG="${1:-}"
[ -n "$TAG" ] || {
	echo "使い方: ./scripts/release.sh v0.1.0" >&2
	exit 1
}
VERSION="${TAG#v}"

APP_TARGZ=$(ls -1 target/release/bundle/macos/*.app.tar.gz 2>/dev/null | head -1 || true)
SIG_FILE=$(ls -1 target/release/bundle/macos/*.app.tar.gz.sig 2>/dev/null | head -1 || true)
DMG=$(ls -1 target/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)

for f in "$APP_TARGZ" "$SIG_FILE" "$DMG"; do
	[ -n "$f" ] && [ -f "$f" ] || {
		echo "エラー: 配布物がそろっていません。先に ./scripts/build.sh を実行してください。" >&2
		echo "  .app.tar.gz: ${APP_TARGZ:-なし}" >&2
		echo "  .sig:        ${SIG_FILE:-なし}" >&2
		echo "  .dmg:        ${DMG:-なし}" >&2
		exit 1
	}
done

# 公証されていないものを配ると、初回起動で弾かれる。
if ! xcrun stapler validate target/release/bundle/macos/MomReply.app >/dev/null 2>&1; then
	echo "エラー: 公証（staple）されていません。" >&2
	echo "./scripts/build.sh を最後まで通してから実行してください。" >&2
	exit 1
fi

OUT=target/release/bundle/latest.json
SIG=$(cat "$SIG_FILE")
NOTES=$(git tag -l --format='%(contents)' "$TAG" 2>/dev/null || true)
[ -n "$NOTES" ] || NOTES="$TAG"

# Intel Mac は対象外。darwin-aarch64 だけを載せる。
# darwin-x86_64 を書くと、Intel 機が落ちてくるものを掴んで動かない。
python3 - "$VERSION" "$SIG" "$(basename "$APP_TARGZ")" "$NOTES" > "$OUT" <<'PY'
import json, sys, datetime
version, sig, name, notes = sys.argv[1:5]
base = "https://github.com/kiyohken2000/momreply/releases/download/v" + version
print(json.dumps({
    "version": version,
    "notes": notes,
    "pub_date": datetime.datetime.now(datetime.UTC).isoformat().replace("+00:00", "Z"),
    "platforms": {
        "darwin-aarch64": {"signature": sig, "url": f"{base}/{name}"},
    },
}, ensure_ascii=False, indent=2))
PY

echo "==> latest.json"
cat "$OUT"

echo
echo "==> アップロード"
gh release create "$TAG" \
	"$DMG" "$APP_TARGZ" "$SIG_FILE" "$OUT" \
	--title "$TAG" --notes "$NOTES"

echo
echo "公開しました。既存の利用者には、設定タブの「更新を確認」から見えます。"
