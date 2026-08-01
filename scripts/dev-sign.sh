#!/bin/sh
# 開発ビルドに安定した署名を与える。
#
# cargo が作るバイナリは ad-hoc 署名で、リンクのたびに cdhash が変わる。
# Keychain のアクセス許可は cdhash に紐づくため、「常に許可」を押しても
# 次のビルドで無効になり、毎回ダイアログが出る（仕様書 7.5.7）。
#
# 証明書で署名すると、許可は証明書に紐づくようになり、再ビルドしても
# 保持される。使う証明書はこの順で決める。
#
#   1. 環境変数 MOMREPLY_SIGN_IDENTITY
#   2. login キーチェーンにあるコード署名用の証明書（最初の 1 つ）
#
# 証明書が 1 つも無い場合は、キーチェーンアクセス.app で作成する。
#   メニュー「キーチェーンアクセス」→「証明書アシスタント」→「証明書を作成...」
#   名前: MomReply Dev / 固有名のタイプ: 自己署名ルート / 証明書のタイプ: コード署名
#
# 使い方:
#   cargo build && ./scripts/dev-sign.sh
set -eu

if [ -n "${MOMREPLY_SIGN_IDENTITY:-}" ]; then
	IDENTITY="$MOMREPLY_SIGN_IDENTITY"
else
	IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null |
		sed -n 's/.*"\(.*\)"/\1/p' | head -1)
fi

if [ -z "$IDENTITY" ]; then
	echo "コード署名に使える証明書がありません。" >&2
	echo "このスクリプト冒頭の手順で作成してください。" >&2
	exit 1
fi

echo "署名に使う証明書: $IDENTITY"

sign() {
	[ -e "$1" ] || return 0
	codesign --force --sign "$IDENTITY" "$1"
	echo "  署名: $1"
}

sign target/debug/momreply-cli
sign target/debug/momreply
sign "target/debug/bundle/macos/MomReply.app"

echo
echo "初回のみ Keychain のダイアログが出ます。「常に許可」を押してください。"
echo "以後は再ビルドしても保持されます。"
