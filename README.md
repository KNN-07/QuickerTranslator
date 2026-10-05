# QuickTranslator

Independent Tauri 2 desktop implementation of the Chinese/Japanese → Vietnamese dictionary-assisted editor workflow. Vietnamese is the target language; Vietnamese and English are interface locales. Offline readings/glosses are lookup assistance, not fluent machine translation.

## Development

Requires Node ≥22.12, npm ≥10, Rust ≥1.90, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). Linux uses GTK3 and WebKitGTK4.1. The installed application needs no server process.

```sh
npm install
npm run data:prepare
npm run build
npm test -- --run
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri -- dev
npm run tauri -- build
```

The npm and Cargo lockfiles pin resolved dependencies. Current published Lindera 6.2.0 embeds IPADIC and implements the `load_dictionary("embedded://ipadic")` / normal-mode `Segmenter` API described in the inspected development documentation; version 7 was not published when implementation began.

## Offline resources

`src-tauri/resources/dictionaries` contains the complete usable Chinese/Japanese subsets of the recorded Vietnamese Wiktionary extraction and all Unihan `kVietnamese` readings. `manifest.json` records source URLs, SHA-256 digests, snapshots, transformations, entry counts and unresolved aliases. Missing glosses are not invented. Ordinary builds use checked-in derived data and need no dictionary-host access.

`npm run data:prepare` verifies and reuses existing resources offline. Explicit refresh: `npm run data:prepare -- --refresh`. Original local input copies may be supplied with `--wiktextract PATH --unihan PATH`; locked checksums still apply without refresh. Raw download cache is outside the repository. Missing resources are a build error, not a fallback to sample dictionaries.

Derived Wiktionary data is CC BY-SA 4.0, separately from application code. Source-page attribution, Unicode V3, Lindera MIT and NAIST/ICOT IPADIC notices are bundled. The reference C# repository has no repository-wide license; none of its implementation, docking library or artwork is copied.

## Native boundaries

One native window per document. IPC text ranges are half-open UTF-16 code-unit offsets; native engines use UTF-8 maps that reject split scalars/surrogates. CPU translation work uses immutable revisioned snapshots and blocking workers. Filesystem, provider HTTP and credentials stay in Rust. Browser-only development explicitly disables native operations and provides no mock translations. Capabilities grant bundled document windows only the required local dialog, clipboard and window operations.

## Dictionaries and migration

Config → Dictionaries provides search, create/edit/delete, language/name-kind choice, optional Japanese readings/POS, per-entry history, coverage and provenance. Config → Data sources/licenses exposes bundled attribution as text. `Reload Dicts` opens previews rather than silently importing.

Select `Dictionaries.config` or individual files. Relative paths resolve against the selected config, including legacy backslashes. Missing/foreign-drive paths are listed for explicit remapping; resolved optional files can be imported separately. Previews show decoded text, encoding, accepted/duplicate/malformed counts and line diagnostics before an atomic commit. BOMs are authoritative; invalid bytes never silently become replacement characters. UTF-16 and legacy Chinese/Japanese/Vietnamese encodings support explicit overrides.

`user-data.sqlite3` stores imported layers, edits, tombstones, metadata, history and shortcuts. Imports/reloads preserve edits and deletions. Immutable bundled dictionaries are never rewritten. Primary Names outrank secondary Names, then VietPhrase; auxiliary meaning dictionaries are lookup-only. User-provided corpora remain local.

Exports require a selected destination, never overwrite imports automatically, and use UTF-8 legacy text. Unrepresentable native multiline/equal-sign values produce an error without touching the destination. Japanese reading/POS metadata stays in SQLite; legacy key/value export contains meanings only. Shortcut imports lowercase keys, keep the first duplicate, and export by descending key length then lexical key.

Corrupt/unsupported user databases are not removed. Bundled lookups remain available; Config reports the unavailable store. Explicit recovery first writes no-overwrite DB/WAL/SHM/settings backups to the chosen destination, selects a new app-local database and retains original files.

## Aligned Chinese review

Source edits translate after 300 ms idle, outside IME composition. Clipboard translation and Re-Translate are explicit actions. Config selects `longest`, `leftToRight` or `longestConditional`, name priority and bracket wrapping. Matching uses a per-revision prefix index with a 20-Unicode-scalar phrase limit and separately compiled Luật Nhân rules. Chinese punctuation/spacing/capitalization changes generated outputs only; ignored spans keep their source mappings.

Click a generated token to select its source and inspect ordered meanings, readings and provenance in Nghĩa. Choosing an alternative or dragging a first-meaning token edits a separate dirty draft. Alt+Shift+Left/Right also moves tokens within the same paragraph; Alt+Left/Right remains reserved for document navigation. Retranslation asks Discard/Cancel before removing draft edits. Stale drafts remain copyable without falsely aligning them to changed source.

Nghĩa inserts a chosen meaning/reading at the retained Vietnamese caret or selection as one undoable edit; explicit override actions save local dictionary changes. Dictionary/model-looking markup is inserted as literal text, not HTML. Source/language/dictionary changes never automatically replace Việt. Source undo survives hiding/reopening panes and resetting layout.

Internal tabs/groups and tokens use pointer dragging to coexist with Tauri's native file-path drop handler. Dockview handles docking itself; no custom docking engine is used. Config adjusts per-pane font sizes, wrapping and synchronized source/output scrolling. Manual Vietnamese text is never given false word-level source alignment.

## Japanese assistance

Choose Japanese explicitly; shared Han characters never cause automatic language detection. Embedded IPADIC normal-mode segmentation supplies source byte spans, base forms, readings and POS. Display readings convert katakana to hiragana. User Japanese phrases/names match longest exact spans on token boundaries; otherwise lookup proceeds surface → lemma → kana reading. Japanese never applies Chinese rules, ignored phrases, Hán Việt readings or punctuation/case conversion.

The panes become Nhật, Cách đọc, Nghĩa từ and Nghĩa từ một nghĩa. Unknown vocabulary remains original text with an explicit unknown indicator in Nghĩa; punctuation, whitespace and emoji pass through without unknown warnings. Switching language cancels stale work and retains Việt.

Recorded bundled-data smoke for `私は学校で日本語を勉強しています。🙂`: 4 of 11 lexical tokens had Vietnamese glosses (`私`, `日本語`, `を`, `い`). `学校` had IPADIC reading `がっこう` but no bundled gloss; `し` exposed lemma `する` and reading `し`, without a bundled gloss. User imports added exact school/Japanese-language glosses and lemma-based `làm`. Homograph glosses are not grammatical disambiguation; use them for review or request explicit AI translation.

## Documents, exports and recovery

File supports Open, Save, Save As, Export and Close. Native titles show filename and dirty state. Open/New/Close/sibling navigation share asynchronous Save/Discard/Cancel decisions; cancelling Save As does not authorize a transition. New opens another native window and does not erase the still-open old document. Native file drops use the same import and dirty guard.

New saves use `.qtp`: UTF-8 JSON with `format:"quicktranslator-project"`, `version:1`, explicit source language, target `vi`, source text, validated Tiptap target JSON, exact-snapshot draft edits, view state and optional original RTF. Keys and global dictionary databases are not included. Existing invalid/future projects are protected from overwrites; atomic-save failure keeps prior bytes and editor work.

Import `.qt`, plain text and local HTML through encoding previews/overrides. `.qt` marker parsing rejects ambiguous structure; invalid scroll indices reset to zero with warnings. RTF group/control parsing preserves supported text formatting, escaped characters, code pages, Unicode/surrogates, paragraphs and field display text. Objects/pictures/field instructions and unsupported layout are omitted with warnings; the original decoded RTF remains available for unchanged original-RTF export. Local HTML imports visible body text without executing or loading scripts/styles/resources. Binary `.doc` (and unsupported `.docx` input) requires conversion to TXT/HTML.

Export current target as UTF-8 TXT, escaped standalone HTML or DOCX. HTML/DOCX support ordered source/readings/full-phrase/first-meaning/target columns and blank-line spacing. RTF export is explicitly the original retained payload, not an export of later target edits.

App-data `settings.json` stores versioned nonsecret layout/interface/editor preferences, traversal bindings, nine snippets and provider metadata. Invalid/future settings remain untouched while safe defaults load; explicit replacement retains `settings.invalid-<UUID>.json`. Unsaved work uses UUID-per-document recovery files; startup offers individual restore/discard choices. Discarding one record leaves other records intact.

Keyboard: Ctrl/Cmd+O/S/Shift+S/E/F, platform undo/redo, Ctrl/Cmd+Shift+N new window, Alt+Left/Right sibling navigation. Configurable Ctrl+K/J words, M/I lines, N/U paragraphs retain the legacy traversal convention; Ctrl+0 inserts a reading, Ctrl+1…6 a meaning, F1…F9 a configured snippet. Typed target shortcuts expand case-insensitively only on a typed delimiter as an isolated undo transaction, never during IME, paste or drop.

## Optional AI translation

`AI Translate` reveals the AI tab; it does not send data. Config → AI profiles supplies official URL presets or custom API roots. Enter a required model ID, choose protocol/authentication and save the profile. Presets intentionally contain no selected model. The default output limit is 4096 tokens. Chat profiles explicitly choose `max_tokens`, `max_completion_tokens` or `omit`; compatible servers receive no requested usage extension.

| Protocol | Appended route | Native authentication |
|---|---|---|
| OpenAI Responses | `/responses` | Bearer API key |
| OpenAI-compatible Chat | `/chat/completions` | Bearer API key, or explicit no authentication |
| Gemini | `/models/{model}:streamGenerateContent?alt=sse` or `:generateContent` | `x-goog-api-key` |
| Anthropic Messages | `/messages` | `x-api-key` and `anthropic-version: 2023-06-01` |

Custom roots retain version/team/proxy prefixes and native provider formats. Gemini strips an optional `models/` prefix and encodes the remaining model as one path segment. Streaming can be disabled per profile. HTTP is accepted automatically only on loopback; other HTTP roots require explicit insecure-transport consent. TLS verification stays enabled; redirects, automatic paid retries and provider/model/protocol fallbacks are disabled.

Translate selected source or the document; improve only selected Vietnamese with explicitly selected source context, which may be empty. Review the actual native payload, endpoint/model, UTF-16 source span, matched glossary and chunk count, then use the separate Send action. Glossaries use the active offline matching policy and include only matches inside each chosen fragment, never complete private dictionaries or surrounding document text. Document requests are sequential chunks of at most 4000 Unicode scalars; paragraph/sentence boundaries and inter-chunk delimiters are retained.

Output streams into preview only. Cancel, refusals, malformed/interrupted responses and output-limit truncation never enable Apply; validated partial prose and completed chunks remain copyable, including non-stream token-limit failures. Only full success with unchanged source/target revisions can apply at the retained target selection/caret. Replace All requires an asynchronous confirmation; either application is one undoable edit. Editing source, switching language, changing matching options/dictionaries or closing the window invalidates native work. Target edits preserve preview but disable stale application/retry. AI never starts on typing, clipboard operations, file open or dictionary import.

Connection Test shows the configured endpoint/model and sends only the fixed `你好。 → vi` request through the actual adapter. It can incur provider charges; it never uploads the open document. Live official-provider access requires the owner's credentials and available model; deterministic loopback tests are not proof of paid-provider connectivity.

## Credentials

Keys are entered transiently, passed once to Rust and cleared from the input. They are neither returned to JavaScript nor stored in browser storage, settings, projects or plaintext fallback files. The native OS store uses service `io.github.quickertranslator.desktop`, bound to profile ID and canonical API root. Changing root/authentication invalidates the active key binding. Credential writes also carry the reviewed endpoint and reject a concurrent endpoint change before storing anything.

If the OS vault is unavailable or locked, explicitly choose **session-only** and re-enter the key. Session keys are zeroizing native-memory values, lost on exit. Deleting a profile removes tracked credential items for all its older endpoints; a locked/unavailable store must be restored before existing OS items can be deleted safely. Linux needs an available Secret Service. The real Linux vault round-trip, endpoint invalidation and deletion were exercised with an isolated GNOME Keyring service; this does not unlock or configure the user's desktop vault.

## Packages and CI

Linux package build:

```sh
npm ci
npm run data:prepare -- --verify
npm run tauri -- build --bundles deb,appimage -- --locked
```

Installers are under `src-tauri/target/release/bundle/`: Debian `.deb` and executable `.AppImage`. AppImage needs executable permission and a compatible Linux desktop; use a local CJK font such as Noto Sans CJK for readable source glyphs. These locally built packages are unsigned and reflect this host's system-library baseline.

`.github/workflows/desktop.yml` checks locked dependencies/resources, TypeScript/Vitest and Rust on native Windows x86_64, Ubuntu 22.04 x86_64, macOS arm64 and macOS Intel runners. It uploads NSIS, deb/AppImage and app/dmg packages with architecture/signing labels, without publishing a release or installing an updater. The workflow passed actionlint; consult GitHub Actions for native platform build results.

Signing is optional and secrets are supplied only on default-branch push/manual builds, never PRs. Windows uses `WINDOWS_CERTIFICATE` (base64 PFX including private key) plus `WINDOWS_CERTIFICATE_PASSWORD`. macOS uses `APPLE_CERTIFICATE` (base64 Developer ID P12) plus `APPLE_CERTIFICATE_PASSWORD`; notarization additionally requires `APPLE_ID`, app-specific `APPLE_PASSWORD` and `APPLE_TEAM_ID`. Incomplete secret sets fail instead of silently downgrading. macOS without owner credentials is explicitly labeled unsigned/ad-hoc; Gatekeeper approval remains necessary. Signing/notarization were not locally verified.

Both built Linux packages were launched on Xvfb with a separate network namespace and no outbound routes. Chinese lookups and Japanese IPADIC readings/glosses worked; AppImage restart restored layout and individually recovered the Japanese document. Native/browser UI checks covered 1280×800 and 900×600. Four real native protocol adapters passed streaming/non-stream loopback smoke, cancellation, selected-text privacy, stale apply guards, one-undo application and per-window job isolation. Official live-provider connectivity remains unverified without owner credentials.


