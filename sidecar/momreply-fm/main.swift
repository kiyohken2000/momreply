// Apple Intelligence（オンデバイス）で 1 回だけ生成する。
//
// # これはアプリに繋がっていない
//
// 動きはする。ビルドも通り、--check も生成も返す。それでも本体からは
// 呼んでいない。**本番と同じプロンプトが 10 回中 10 回、安全機構に
// 拒否されたため。**
//
// 引き金はプロンプトに含まれる会話履歴だった。受信メッセージが穏当でも、
// 「# 直近の会話」に強い言葉が入っていれば拒否される。このアプリは
// 履歴を必ず入れるので、実質いつも拒否される。
//
// 履歴を外せば通るが、そのときの出力も使えなかった。書いていない
// 出来事を作り（「仕事で大変だったんだよね」）、システム指示の文を
// そのまま返信に混ぜてきた。
//
// 設定に出して選べるようにすると、選んだ人は「返信が来ない」ことだけを
// 見ることになる。いちばん分かりにくい壊れ方なので出さない。
//
// 残してあるのは、モデルが変われば結論も変わるため。そのときは
// llm/apple.rs を書いてここを呼べばよい。
//
// # なぜ別プロセスか
//
// FoundationModels は Swift でしか呼べない。Rust から使うには橋渡しが要る。
// 常駐させずに 1 回ごとに起動して終わる形にしてある。状態を持たないので、
// 落ちても本体に波及しない。
//
// # 入出力
//
// stdin  {"system": "...", "messages": [{"role":"user|assistant","content":"..."}],
//         "temperature": 0.8, "max_tokens": 700}
// stdout {"text": "..."}  または  {"error": "..."}
//
// 終了コードは常に 0。エラーも JSON で返す。呼び出し側が
// 「落ちた」と「モデルが断った」を区別できるようにするため。

import Foundation
import FoundationModels

struct Message: Decodable {
    let role: String
    let content: String
}

struct Request: Decodable {
    let system: String?
    let messages: [Message]
    let temperature: Double?
    let maxTokens: Int?

    enum CodingKeys: String, CodingKey {
        case system, messages, temperature
        case maxTokens = "max_tokens"
    }
}

func emit(_ object: [String: String]) -> Never {
    let data = try! JSONSerialization.data(withJSONObject: object, options: [])
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
    exit(0)
}

func fail(_ message: String) -> Never {
    emit(["error": message])
}

func textSegment(_ text: String) -> Transcript.Segment {
    .text(Transcript.TextSegment(content: text))
}

@main
struct MomReplyFM {
    static func main() async {
        // --check だけ渡されたら、使えるかどうかだけ返す。
        // キーの検証と同じ位置づけで、生成せずに可否を知りたい。
        let checkOnly = CommandLine.arguments.contains("--check")

        switch SystemLanguageModel.default.availability {
        case .available:
            if checkOnly { emit(["text": "available"]) }
        case .unavailable(let reason):
            switch reason {
            case .deviceNotEligible:
                fail("この Mac は Apple Intelligence に対応していません")
            case .appleIntelligenceNotEnabled:
                fail("システム設定で Apple Intelligence を有効にしてください")
            case .modelNotReady:
                fail("モデルの準備中です。しばらく待ってから試してください")
            @unknown default:
                fail("Apple Intelligence を利用できません")
            }
        @unknown default:
            fail("Apple Intelligence の状態を判断できません")
        }

        let input = FileHandle.standardInput.readDataToEndOfFile()
        guard let request = try? JSONDecoder().decode(Request.self, from: input) else {
            fail("入力を読めませんでした")
        }
        guard let last = request.messages.last, last.role == "user" else {
            fail("最後のメッセージが user ではありません")
        }

        // 文体の手本を会話として積む。まとめて指示文に入れるより、
        // 実際のやり取りの形にしたほうが模倣の精度が高い（仕様書 8.2）。
        var entries: [Transcript.Entry] = []
        if let system = request.system, !system.isEmpty {
            entries.append(
                .instructions(
                    Transcript.Instructions(
                        segments: [textSegment(system)], toolDefinitions: [])))
        }
        for m in request.messages.dropLast() {
            switch m.role {
            case "user":
                entries.append(.prompt(Transcript.Prompt(segments: [textSegment(m.content)])))
            case "assistant":
                entries.append(
                    .response(
                        Transcript.Response(assetIDs: [], segments: [textSegment(m.content)])))
            default:
                break
            }
        }

        let session = LanguageModelSession(transcript: Transcript(entries: entries))
        let options = GenerationOptions(
            temperature: request.temperature,
            maximumResponseTokens: request.maxTokens)

        do {
            let response = try await session.respond(to: last.content, options: options)
            emit(["text": response.content])
        } catch let error as LanguageModelSession.GenerationError {
            // 断られた理由を残す。ガードレールに触れたのか、入りきらな
            // かったのかで、呼び出し側の直し方が変わる。
            switch error {
            case .exceededContextWindowSize:
                fail("プロンプトがモデルの上限を超えました")
            case .guardrailViolation:
                fail("Apple Intelligence の安全機構が生成を拒否しました")
            default:
                fail("生成に失敗しました: \(error)")
            }
        } catch {
            fail("生成に失敗しました: \(error)")
        }
    }
}
