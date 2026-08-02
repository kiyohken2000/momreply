#!/bin/sh
# 配布用の .app と .dmg を作る。
#
# # 署名について
#
# cargo tauri build が作る .app は ad-hoc 署名で、**ビルドのたびに
# cdhash が変わる**。Keychain のアクセス許可は cdhash に紐づくため、
# 「常に許可」を押しても次のビルドで無効になり、API キーを使うたびに
# ダイアログが出る（仕様書 7.5.7）。
#
# 手元のコード署名証明書があれば、それで署名し直す。許可が証明書に
# 紐づくようになり、再ビルドしても保持される。証明書が無くても
# ビルドは通る（ダイアログが毎回出るだけ）。
#
# 配布そのものには署名は要らない。このリポジトリはソースで配る形なので、
# 使う人は自分の環境でビルドし、自分の証明書で署名する。
#
# 使い方:
#   ./scripts/build.sh
set -eu

cd "$(dirname "$0")/.."

echo "==> ビルド"
cargo tauri build

APP="target/release/bundle/macos/MomReply.app"
[ -d "$APP" ] || {
	echo "エラー: $APP が作られていません" >&2
	exit 1
}

if [ -n "${MOMREPLY_SIGN_IDENTITY:-}" ]; then
	IDENTITY="$MOMREPLY_SIGN_IDENTITY"
else
	IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null |
		sed -n 's/.*"\(.*\)"/\1/p' | head -1)
fi

if [ -n "${IDENTITY:-}" ]; then
	echo
	echo "==> 署名: $IDENTITY"
	codesign --force --deep --sign "$IDENTITY" "$APP"
else
	echo
	echo "コード署名証明書が見つかりません。ad-hoc 署名のままにします。"
	echo "この場合、ビルドし直すたびに Keychain のダイアログが出ます。"
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
