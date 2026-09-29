# Rust デスクトップ GUI 比較: AI チャットアプリ

同一仕様の AI チャットアプリを 5 つのフレームワークで実装し、開発体験・性能・配布サイズを比較する。
ロジック（LLM クライアント、永続化、設定、Markdown 解析）はすべて `crates/chat-core` に集約し、
各アプリは UI とイベントループへの橋渡しだけを持つ。

| App    | Crate         | Version | UI 記述                      | 描画                  |
| ------ | ------------- | ------- | ---------------------------- | --------------------- |
| egui   | `chat-egui`   | 0.36    | Rust（イミディエイトモード） | wgpu / glow           |
| Tauri  | `chat-tauri`  | 2.9     | HTML + CSS + TypeScript      | OS の WebView         |
| iced   | `chat-iced`   | 0.14    | Rust（Elm アーキテクチャ）   | wgpu / tiny-skia      |
| Slint  | `chat-slint`  | 1.18    | `.slint` DSL + Rust          | femtovg / software 等 |
| Dioxus | `chat-dioxus` | 0.7     | `rsx!` + CSS（React 風）     | OS の WebView (wry)   |

## 計測値

計測はモック provider（`CHAT_COMPARE_MOCK_DELAY_MS` 間隔で固定レスポンスを 1 トークンずつ送る）で行い、ネットワーク差を排除する。

### ビルド・サイズ（`bench/measure.sh`）

最新の結果は `bench/results/` を参照。

<!-- bench:start -->

Linux x86_64 / 4 vCPU / rustc 1.97.1、`profile.release` は `strip = true`, `lto = "thin"`（2026-09-29）。

| App    | Release binary | Clean build (s) | Incremental build (s) |
| ------ | -------------: | --------------: | --------------------: |
| egui   |          29 MB |           192.1 |                  73.3 |
| Tauri  |          17 MB |           262.0 |                  76.5 |
| iced   |          25 MB |           211.7 |                  63.1 |
| Slint  |          33 MB |           211.7 |                  75.4 |
| Dioxus |          15 MB |           142.8 |                  32.4 |

- Clean build は `target/release` を消してからアプリ単体をビルドした時間。Incremental は `chat-core` を touch した後の再ビルド。
- 今回の Tauri / iced の計測中は別のビルドが並行して走っていたため、時間は参考値。再計測は `bench/measure.sh`。
- WebView 系（Tauri / Dioxus）は WebView を OS に依存するためバイナリが小さい。インストーラサイズは未計測。

<!-- bench:end -->

### 実行時（手動計測）

| App    | 起動（cold / warm） | アイドル時メモリ | 1,000 メッセージ表示時メモリ | ストリーミング中 CPU | カクつき |
| ------ | ------------------- | ---------------- | ---------------------------- | -------------------- | -------- |
| egui   | TBD                 | TBD              | TBD                          | TBD                  | TBD      |
| Tauri  | TBD                 | TBD              | TBD                          | TBD                  | TBD      |
| iced   | TBD                 | TBD              | TBD                          | TBD                  | TBD      |
| Slint  | TBD                 | TBD              | TBD                          | TBD                  | TBD      |
| Dioxus | TBD                 | TBD              | TBD                          | TBD                  | TBD      |

手順:

1. `export CHAT_COMPARE_HOME=$(mktemp -d)` で専用のデータディレクトリを使う
2. `echo '{"provider":"mock"}' > $CHAT_COMPARE_HOME/settings.json`
3. 長い会話は `cargo run -p chat-core --example seed -- 1000` で生成する
4. リリースビルドを起動し、RSS（`ps -o rss`, Activity Monitor, タスクマネージャ）と CPU を記録する。
   WebView 系（Tauri / Dioxus）は WebView プロセスの分も合算する

## 定性評価（実装時のメモ）

### 状態管理とストリーミング

| App    | ストリームの受け取り方                                                                       | 所感                                                                                   |
| ------ | -------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| egui   | tokio タスクで受信 → `std::sync::mpsc` へ転送し `ctx.request_repaint()`、`App::logic` で反映 | 状態は普通の `&mut self`。毎フレーム描画なので Markdown 解析結果のキャッシュが必要     |
| Tauri  | Rust 側で `Mutex` 状態に反映し、HTML 化した最新メッセージを `emit("stream")`                 | コマンド / イベント / TS の 3 層に分かれ、型定義を Rust と TS で二重に持つ             |
| iced   | `Task::run(stream, Message::Stream)` で受信チャネルをそのままメッセージ化                    | 最も素直。`view` が純粋関数なので解析キャッシュは `update` 側で更新する                |
| Slint  | `slint::spawn_local` で UI スレッド上から tokio チャネルを直接 await                         | `Rc<RefCell<_>>` と `VecModel` の手動同期が必要。最後の 1 行だけ `set_row_data` で更新 |
| Dioxus | `spawn` で受信し `Signal` を書き換え                                                         | React 的で簡潔。props 比較で再描画はストリーミング中のメッセージだけに限定できる       |

### テキスト入力・IME・選択/コピー・スクロール

- **Enter 送信 / Shift+Enter 改行**: 全アプリで実装。
  - egui: 複数行 `TextEdit` が先に改行を挿入するので、送信時に末尾の改行を取り除く。
  - iced: `text_editor::key_binding` で Enter を `Binding::Custom(Send)` に差し替え。
  - Slint: `FocusScope::capture-key-pressed` で `TextEdit` より先に横取り。
  - Dioxus / Tauri: `keydown` で `isComposing`（IME 変換確定の Enter）を除外する必要がある。
- **Dioxus の制御入力**: `textarea { value: "{signal}" }` にすると高速入力で文字が欠落した
  （"Hello dioxus" → "Hll ixs"）。WebView への DOM 更新が非同期なため古い値で上書きされる。
  入力欄は非制御（`initial_value` / `eval` でクリア）にして回避。
- **選択/コピー**: egui のラベルは標準で選択可能。iced 0.14 のテキストは選択できないため、コードブロックに Copy ボタンを付けた。
  Slint は Rust API にクリップボードがないため、読み取り専用 `TextInput` で選択可能にした。WebView 系はブラウザ標準。
- **自動スクロール**: egui `ScrollArea::stick_to_bottom`、iced `scrollable().anchor_bottom()`、
  Slint は `changed content-height` で `content-y` を補正、WebView 系は CSS `flex-direction: column-reverse`。
- **日本語フォント**: egui は CJK フォントを同梱しないため OS のフォントファイルを探して読み込む必要がある。他はシステムフォントにフォールバックする。

### Markdown / コードブロック表示

- `chat-core::markdown::parse` がフラットなブロック列（見出し・段落・リスト項目・引用・コード・罫線）を返し、ネイティブ系 3 つはこれを描画する。
  WebView 系は `markdown::to_html`（生 HTML はエスケープ、http(s)/mailto 以外のリンクは無効化）を `innerHTML` に入れる。
- egui: `LayoutJob` でインライン装飾を組み立てる。
- iced: `rich_text` + `span` でインライン装飾が素直に書ける（`markdown` feature の組み込みウィジェットもある）。
- Slint: `StyledText`（`StyledText::from_markdown`）はインライン装飾のみで見出し・コードブロック非対応。
  ブロック構造は `.slint` 側で分岐し、インラインは再エスケープした Markdown を渡している。汎用 `monospace` フォント名がないため `Platform.os` で分岐。
- WebView 系は CSS でそのまま整形でき、手間は最小。

### ビルド・依存関係・配布

- 5 フレームワークを 1 つの Cargo workspace に置くと、`webkit2gtk-sys`（`links = "web_kit2"`）が 1 バージョンしか共存できない。
  dioxus-desktop 0.7 が wry 0.53 を要求するため Tauri は 2.9 系に固定される（`apps/tauri/src-tauri/Cargo.toml` のコメント参照）。
  Tauri は内部クレート（`tauri-runtime` 等）と npm の `@tauri-apps/api` の minor も揃える必要がある。
- Tauri の `generate_context!` は `frontendDist` の存在を要求するため、`build.rs` で空ディレクトリを作って `cargo build` 単体でも通るようにしている。
  デバッグビルドは dev サーバ（`pnpm tauri dev`）前提で、埋め込みアセットを使うには `--features tauri/custom-protocol`。
- Slint は `build.rs` で `.slint` をコンパイルする。生成コードがワークスペースの clippy 方針（`unwrap_used` 禁止）に違反するため、生成モジュールだけ lint を許可。
- Slint のライセンスは GPL-3.0 / ロイヤリティフリー / 商用の三択（`deny.toml` でロイヤリティフリーを許可）。
- 5 つの GUI スタックのデバッグ情報は合計数十 GB に達したため、依存クレートの `debug = false` を設定。

### コード量

`tokei` の code 行数（Cargo.toml は除く。テストを含む chat-core は別枠）。最新値は `bench/measure.sh` の結果を参照。

| App               | Rust | その他                      | 合計 |
| ----------------- | ---: | --------------------------- | ---: |
| chat-core（共通） | 1428 | -                           | 1428 |
| egui              |  479 | -                           |  479 |
| iced              |  496 | -                           |  496 |
| Slint             |  337 | Slint 401                   |  738 |
| Dioxus            |  346 | CSS 212                     |  558 |
| Tauri             |  217 | TS 199, CSS 226, HTML 60 他 |  770 |
