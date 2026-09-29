# rust-desktop-chat-compare

同じ「AI チャットアプリ」を Rust の 5 つのデスクトップ GUI フレームワークで実装し、
開発体験・性能・配布サイズを比較する検証用 monorepo。結果と所感は [docs/comparison.md](docs/comparison.md)。

| Path                  | 内容                                                                  |
| --------------------- | --------------------------------------------------------------------- |
| `crates/chat-core`    | UI 非依存のロジック（LLM ストリーミング、永続化、設定）               |
| `apps/egui`           | egui (eframe)                                                         |
| `apps/tauri`          | Tauri v2（`src-tauri/` + Vite/TypeScript フロント）                   |
| `apps/iced`           | iced                                                                  |
| `apps/slint`          | Slint                                                                 |
| `apps/dioxus`         | Dioxus desktop                                                        |
| `apps/mcp-app-viewer` | MCP Apps の HTML View を表示する Web ホスト（比較対象外の補助ツール） |
| `bench/`              | 計測スクリプトと結果                                                  |

## 機能（全アプリ共通）

- 会話一覧サイドバー + チャット画面 + 入力欄（Enter で送信、Shift+Enter で改行）
- Anthropic Messages API のストリーミング表示と途中キャンセル
- Markdown 表示（見出し・リスト・コードブロック等）
- 会話履歴を OS 標準のデータディレクトリに JSON で保存
- 設定画面（provider・API キー・モデル・system prompt・max tokens）。API キーは OS キーチェーンに保存

## 実行

```bash
mise install                 # ツールチェイン
pnpm install                 # Tauri フロントエンドの依存

cargo run -p chat-egui
cargo run -p chat-iced
cargo run -p chat-slint
cargo run -p chat-dioxus
pnpm --filter chat-tauri tauri dev
pnpm --filter mcp-app-viewer dev   # MCP Apps viewer: http://localhost:5180/
```

Linux では GUI 系のシステムライブラリが必要: `scripts/install-linux-deps.sh`

設定画面で provider を **Mock** にすると API キーなしで固定レスポンスがストリーミングされる。

| 環境変数                     | 用途                                              |
| ---------------------------- | ------------------------------------------------- |
| `CHAT_COMPARE_HOME`          | データ・設定の保存先を上書き（計測・テスト用）    |
| `CHAT_COMPARE_MOCK_DELAY_MS` | Mock provider のトークン間隔（既定 20ms）         |
| `RUST_LOG`                   | ログレベル（`tracing-subscriber` の `EnvFilter`） |

## 開発

```bash
mise run fmt   # フォーマット
mise run lint  # lint / ポリシーチェック
mise run test  # テスト
mise run ci    # 必須チェック一式
```
