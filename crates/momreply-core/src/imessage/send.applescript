-- 本文は必ず argv で受け取る。文字列連結でスクリプトを組み立てないこと。
-- 改行・引用符・絵文字で壊れる（仕様書 14.6）。
on run argv
	set targetBuddy to item 1 of argv
	set targetMessage to item 2 of argv
	tell application "Messages"
		set targetService to 1st account whose service type = iMessage
		set theBuddy to participant targetBuddy of targetService
		send targetMessage to theBuddy
	end tell
end run
