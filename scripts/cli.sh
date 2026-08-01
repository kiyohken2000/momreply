#!/bin/sh
# momreply-cli を「ビルド → 署名 → 実行」の順で動かす。
#
# `cargo run` を使ってはいけない。実行前にリンクし直すため、
# dev-sign.sh で付けた署名が ad-hoc に戻る。すると Keychain の
# アクセス許可が無効になり、毎回ダイアログが出る（仕様書 7.5.7）。
#
# 使い方:
#   ./scripts/cli.sh generate --slug mother
#   ./scripts/cli.sh questions list --slug mother
set -eu

cd "$(dirname "$0")/.."

cargo build -q -p momreply-cli
./scripts/dev-sign.sh >/dev/null

exec ./target/debug/momreply-cli "$@"
