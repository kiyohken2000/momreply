# MomReply

[日本語](README.md)

A macOS menu-bar app that writes replies, with an LLM, to iMessages from a chosen contact.

Drafts can be reviewed before sending, or sent automatically once you allow it.
The contact is picked from the conversation list in `chat.db`.

> **Status: it works. Apple Silicon only.**
> Receive → generate → auto-send → verify the send has run end-to-end on real messages.

## Install

Download the `.dmg` from [Releases](https://github.com/kiyohken2000/momreply/releases/latest)
and drag `MomReply.app` into `Applications`.

It is signed and notarized, so it opens on a double click.
Later updates come from "Check for updates" in the Settings tab.

**Intel Macs are not supported.** Only an Apple Silicon build is published.

If you would rather read the code before installing it, see
[Building from source](#building-from-source). This tool reads your entire
message history and sends parts of it to an LLM, so that path is kept open.

## Screens

| Replies | Per-contact settings |
|---|---|
| ![Replies tab](docs/images/replies.jpg) | ![Contacts tab](docs/images/contacts.jpg) |

| Model and keys | About you |
|---|---|
| ![Settings tab](docs/images/settings.jpg) | ![About-you tab](docs/images/self.jpg) |

Everything lives in one popover from the menu bar. Nothing appears in the Dock.
When nothing is waiting for review, it shows the recent exchange and a button to
draft a reply.

---

## What problem is this for

For messages from family or people close to you, where

- not replying makes the relationship worse, but
- composing a reply every time has become a burden.

The goal is that **the conversation does not go silent while you are not looking.**

So instead of "answer the question", it takes the stance of
**"show that you heard them, but commit to nothing."**

Dates, yes/no, amounts and promises are never stated outright. That is why it
never has to stop and ask you something, and why it is unlikely to send a wrong
fact on your behalf. The cost is that the other person does not get the answer
they actually wanted. The only things it will state outright are the ones you
wrote in `self.md` (below).

---

## Requirements

| | |
|---|---|
| OS | macOS 13 or later (developed and tested on macOS 26.6) |
| CPU | **Apple Silicon only** (Intel Macs are not supported) |
| Rust | 1.95 or later (required by `libsqlite3-sys`) |
| Node.js | 20 or later (used to build the frontend) |
| Xcode Command Line Tools | `xcode-select --install` |
| Permission | Full Disk Access |
| API key | One of Anthropic / Google / OpenAI |

Install Rust with rustup. The Homebrew `rust` formula can be too old.

```sh
brew install rustup
rustup default stable
```

`rust-toolchain.toml` pins the toolchain, so it switches automatically inside
the repository.

Everything below is for **building from source**. To use a prebuilt app, get it
from [Releases](https://github.com/kiyohken2000/momreply/releases/latest).

### Full Disk Access

Required to read `chat.db` (`~/Library/Messages/chat.db`).
**Without it the file exists but cannot be opened, and not a single message is read.**

What you grant it to depends on how you run it.

| | What to add |
|---|---|
| Running the built `.app` | `MomReply.app` |
| Running `cargo tauri dev` | Terminal / VS Code — whatever launches `cargo` |
| Using the CLI | Same as above |

1. System Settings → Privacy & Security → Full Disk Access
2. Add the item from the table above and turn it on
3. **Quit it completely and start it again** (closing the window is not enough)

The app also shows a guide for this. While the permission is missing, that
screen is shown instead of everything else.

---

## Building from source

### 1. Build

```sh
npm install
./scripts/build.sh
```

This produces `target/release/bundle/macos/MomReply.app`.
Copy it to `/Applications` and launch it.

```sh
cp -R target/release/bundle/macos/MomReply.app /Applications/
open /Applications/MomReply.app
```

A speech-bubble icon appears in the menu bar. Nothing appears in the Dock.

> `scripts/build.sh` re-signs the app with a code-signing certificate if you
> have one. The ad-hoc signature that `cargo tauri build` applies changes on
> every build, which drops the Keychain "Always Allow" grant each time.
>
> With a Developer ID certificate and notarization credentials it also
> notarizes and staples. Neither is needed to use it on your own machine.

### 2. Grant Full Disk Access

A guide appears on first launch. Open System Settings from the button, add
`MomReply`, then **quit the app and open it again.**

### 3. Enter an API key

In the Settings tab, enter the key for the provider you want. Saving runs a
connectivity test; once it says "Verified" it is usable.

- Keys are stored **only in the Keychain**, never in a config file or a log
- Only the last 4 characters are ever shown
- No command returns the key itself

### 4. Choose who to reply to

Pick from the conversation list in the Contacts tab. There is no free-text entry.

**Messages received before you add the contact are never processed.**
To prevent mass-replying to old history, the highest ROWID at that moment is
recorded when the contact is added. Style examples are also built at that point
from your past replies to that person.

### 5. Decide how it behaves

| Where | Setting |
|---|---|
| Contacts tab | Reply length (preset or target character count), auto-send |
| Settings tab | Dry run, global auto-send switch, which model to use |
| About-you tab | `self.md` (writing direction, and facts it may state) |

**Dry run is on and auto-send is off by default.** As shipped it only generates
drafts and holds them for review; nothing is sent. Watch it for a while by hand
before changing that.

### What it is doing

| Menu-bar icon | State |
|---|---|
| Bubble with three dots | Idle |
| Outline only | Waiting to see if more messages follow (45s) |
| Filled in | Writing a reply |

### CLI (for inspection)

Uses the same `momreply-core` as the app. Useful for checking behaviour.

```sh
./scripts/cli.sh chats                                  # conversations (bodies are not read)
./scripts/cli.sh messages --handle x@icloud.com --limit 20
./scripts/cli.sh burst --slug someone                   # how consecutive messages get grouped
./scripts/cli.sh target list
./scripts/cli.sh target set --slug someone --preset chars:250
```

### Publishing a release (for maintainers)

```sh
./scripts/build.sh          # sign, notarize, sign the update artifact
./scripts/release.sh v0.1.0 # upload to GitHub Releases
```

`release.sh` builds `latest.json` from the build output. The updater reads that
file. Writing it by hand gets the signature wrong, and the update is then
rejected silently.

**The update signing key (`~/.tauri/momreply.key`) never goes into the repository.**
Only builds signed with it are accepted as updates — and losing it means you can
never ship another version.

---

## Safety design

An auto-send mistake cannot be taken back, so the guards were built first.

- **Never writes to chat.db.** The connection uses `SQLITE_OPEN_READ_ONLY` only,
  and immediately asks SQLite itself whether the handle is read-only, as a second
  check. There is exactly one function that opens it.
- **Messages from before registration are never processed.** The protection lives
  inside the function that registers a contact, which requires a `chat.db`
  connection as an argument, so there is no path around it.
- **Other people's conversations are never loaded.** The allowlist is a SQL
  `WHERE` clause, not a filter applied after reading.
- **Auto-send defaults to off.** Dry run defaults to on.
- One handle cannot belong to two contacts (UNIQUE constraint).

The send-side guards are implemented, and each has been observed firing in real use.

| Guard | Effect |
|---|---|
| Already replied | Checked before generating **and again immediately before sending** |
| Cooldown | 60 seconds since the last send |
| Consecutive auto-replies | Over the limit it drops to review mode; the count can be reset by hand |
| Per hour / per day | Over the limit it drops to review mode |
| Stale | Messages that sat too long are not answered automatically |
| Retraction re-check | If the sender unsends while generating, nothing goes out |
| Send verification | Confirmed against `chat.db`. If it cannot be confirmed, **nothing is resent** |
| Runaway length | Output over the limit is held for review instead of sent |

**The spending limit does not really work right now.** No model prices are
registered, so the computed cost is always 0. The count-based limits are the
only thing holding it back.

---

## Data it touches

| Location | Contents |
|---|---|
| `~/Library/Messages/chat.db` | **Read only.** Only the registered contact's conversation |
| `~/Library/Application Support/net.votepurchase.momreply/app.db` | History, generation log, style examples |
| `.../self.md` | Writing direction, and facts it may state |
| `.../targets/<slug>.md` | Per-contact profile |
| Keychain | API keys |

API keys and message bodies are never written to config files, logs, or the
repository. `self.md` and `app.db` live outside the repository (under
Application Support) and are also blocked by `.gitignore`.

### `self.md`

It has two roles.

1. **Direction for how replies are written.** "Keep it casual", "no emoji", and
   so on. **This takes priority over the style examples.**
2. **Facts it may state.** Replies commit to nothing in general, but anything
   written here can be said outright.

Neither of these can be derived from the other person's profile, however
detailed. Only you have them.

---

## Layout

```
crates/
├── momreply-core/          core
│   └── src/
│       ├── imessage/       chat.db (read-only)
│       ├── store.rs        app.db schema and CRUD
│       ├── pipeline/       prompt, guards, generation, sending
│       ├── llm/            Anthropic / Gemini / OpenAI
│       ├── fewshot.rs      builds style examples from your past replies
│       ├── questions.rs    extracts questions
│       ├── profile.rs      self.md and contact profiles
│       └── paths.rs        file locations
├── momreply-cli/           CLI for inspection and management
src-tauri/                  menu-bar app (thin shell)
src/                        UI (React)
```

All access to `chat.db` goes through `momreply-core::imessage`. There is one
function that opens it, and it cannot open anything but read-only.

---

## Progress

- [x] **Phase 0** Reading chat.db (read-only connection, `attributedBody` decoding, exclusion rules)
- [ ] **Phase 1** Generation + dry run
  - [x] Contact registration and backlog protection
  - [x] LLM providers (Claude / Gemini / OpenAI) — Apple Intelligence not started
  - [x] Style-example extraction
  - [x] Reply generation
- [x] **Phase 2** Sending + menu-bar UI
- [x] **Phase 3** Guards + fully automatic (verified end-to-end on real messages)
- [ ] **Phase 4** Automatic profile updates

Not started or unfinished:

- Apple Intelligence (on-device generation)
- Model prices (the reason the spending limit does nothing)
- Opening the popover by tapping a notification (the plugin has no desktop API for it)
- Intel Mac support (universal binary)

The original design document is `docs/momreply-spec.md`. The implementation has
deliberately diverged from it in places; the differences are listed at the top of
that file.

---

## Implementation notes

macOS's `chat.db` has a few traps.

1. **`message.text` is almost always NULL.** The body comes from
   `attributedBody` (a typedstream). In the environment used for testing it was
   NULL for every message in the target conversation — `text` yielded nothing.
2. **Joining through the `handle` table loses your own messages.** Sent messages
   can have `handle_id = 0`. About two thirds of them did in testing. Go through
   `chat_message_join` → `chat` instead.
3. **Full Disk Access requires restarting the app after granting it.**
4. **Dev builds are re-signed on every link,** so permissions granted to the
   `.app` fall off. While developing, grant them to your terminal or IDE and
   launch from there.

---

## A note on how this reads to the other person

Messages sent by this tool **arrive as if you wrote them.**
The recipient has no way to know they are talking to an AI.

Who you use it with, and whether you tell them, is left to you. The defaults are
auto-send off and dry run on, so that you start by reading what it writes.

---

## License

MIT License. See [LICENSE](LICENSE).
