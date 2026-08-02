#!/bin/sh
# 配布用の .app と .dmg を作る。必要なら署名と公証まで行う。
#
# # 署名の使い分け
#
# 手元で使うだけなら署名は要らない。ただし cargo tauri build が付ける
# ad-hoc 署名は**ビルドのたびに cdhash が変わる**。Keychain のアクセス許可は
# cdhash に紐づくため、「常に許可」を押しても次のビルドで無効になり、
# API キーを使うたびにダイアログが出る（仕様書 7.5.7）。
# 証明書で署名すると許可は証明書に紐づき、再ビルドしても保持される。
#
# 他人に配るなら Developer ID Application 証明書と公証が要る。
# 無いと初回起動で Gatekeeper に「壊れています」と言われる。
#
# 使う証明書はこの順で決める。
#   1. 環境変数 MOMREPLY_SIGN_IDENTITY
#   2. Developer ID Application（配布できる）
#   3. その他のコード署名証明書（手元用）
#   4. 無ければ ad-hoc のまま
#
# # 公証
#
# 事前に一度だけ資格情報を保存する。
#
#   xcrun notarytool store-credentials momreply \
#     --apple-id "you@example.com" \
#     --team-id "TEAMID" \
#     --password "アプリ用パスワード"
#
# アプリ用パスワードは appleid.apple.com で作る。Apple ID の
# ログインパスワードそのものは使えない。
#
# 保存済みなら、このスクリプトが公証と staple まで行う。
# 無ければ署名だけで終える（手元で使うぶんには困らない）。
#
# 使い方:
#   ./scripts/build.sh
set -eu

cd "$(dirname "$0")/.."

NOTARY_PROFILE="${MOMREPLY_NOTARY_PROFILE:-momreply}"

# --- 証明書を決める ---
pick_identity() {
	if [ -n "${MOMREPLY_SIGN_IDENTITY:-}" ]; then
		echo "$MOMREPLY_SIGN_IDENTITY"
		return
	fi
	security find-identity -v -p codesigning 2>/dev/null |
		sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1 && return
}
IDENTITY=$(pick_identity)
DISTRIBUTABLE=no

if [ -z "${IDENTITY:-}" ]; then
	IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null |
		sed -n 's/.*"\(.*\)"/\1/p' | head -1 || true)
else
	DISTRIBUTABLE=yes
fi

if [ -n "${IDENTITY:-}" ]; then
	echo "==> 署名に使う証明書: $IDENTITY"
	export APPLE_SIGNING_IDENTITY="$IDENTITY"
else
	echo "==> コード署名証明書が見つかりません。ad-hoc 署名のままにします。"
	echo "    ビルドし直すたびに Keychain のダイアログが出ます。"
fi

echo
echo "==> ビルド"
cargo tauri build

APP="target/release/bundle/macos/MomReply.app"
DMG=$(ls -1 target/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)
[ -d "$APP" ] || {
	echo "エラー: $APP が作られていません" >&2
	exit 1
}

# --- 署名の中身を確認する ---
#
# 公証は Hardened Runtime とセキュアタイムスタンプを要求する。
# どちらも「付いているつもり」で落ちるので、提出前に必ず見る。
if [ "$DISTRIBUTABLE" = yes ]; then
	echo
	echo "==> 署名を確認"
	codesign -dv --verbose=4 "$APP" 2>&1 |
		grep -E "Authority=Developer ID|TeamIdentifier|flags=|Timestamp=" || true

	if ! codesign -dv --verbose=4 "$APP" 2>&1 | grep -q "runtime"; then
		echo
		echo "Hardened Runtime が付いていないので付け直します。" >&2
		codesign --force --options runtime --timestamp \
			--entitlements src-tauri/entitlements.plist \
			--sign "$IDENTITY" "$APP"
	fi

	codesign --verify --deep --strict --verbose=2 "$APP"
	echo "署名の検証: OK"
fi

# --- 公証 ---
if [ "$DISTRIBUTABLE" = yes ] && [ -n "$DMG" ] &&
	xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1; then
	echo
	echo "==> 公証（数分かかります）"
	xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait

	echo "==> staple"
	# .dmg と .app の両方に貼る。.app だけ取り出して配られても
	# オフラインで検証できるようにする。
	xcrun stapler staple "$DMG"
	xcrun stapler staple "$APP"

	echo
	echo "==> Gatekeeper の判定"
	spctl -a -vvv -t install "$APP" 2>&1 || true
elif [ "$DISTRIBUTABLE" = yes ]; then
	echo
	echo "公証していません。他人の Mac では Gatekeeper に弾かれます。"
	echo "資格情報を保存するには、このスクリプト冒頭の store-credentials を実行してください。"
fi

echo
echo "==> できたもの"
ls -1d target/release/bundle/macos/*.app target/release/bundle/dmg/*.dmg 2>/dev/null

cat <<'EOS'

次の手順:

  1. cp -R target/release/bundle/macos/MomReply.app /Applications/
  2. open /Applications/MomReply.app
  3. フルディスクアクセスを求める画面が出るので、案内どおりに許可する
     （許可のあと、アプリの終了と起動し直しが要ります）

開発中の cargo tauri dev とは別のアプリとして扱われます。
フルディスクアクセスも Keychain の許可も、それぞれに必要です。
EOS
