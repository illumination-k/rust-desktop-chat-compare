# AGENTS Guideline

This repository is pre-alpha and under active development. The API is not stable and may change without a major version bump, so backwards compatibility is not guaranteed at this stage.
So developers of this repository DO NOT need to worry about breaking changes or maintaining backwards compatibility. We prefer to iterate quickly and make breaking changes as needed, rather than trying to maintain backwards compatibility.

## Policy

Follow the YANGI, SOLID, DRY, and KISS principles in all code and documentation. Prioritize simplicity, readability, and maintainability over cleverness or optimization. Avoid premature optimization and over-engineering. Strive for clear and concise code that is easy to understand and modify.

## Development Process

Run `mise install` first to install the toolchain and project tools.

At the end of a session, run `mise run ci` and make sure it passes. Use the narrower tasks while iterating:

```bash
mise run fmt      # Format
mise run lint     # Lint and policy checks
mise run test     # Tests
mise run ci       # Full required verification
```

## Commands

Run `mise install` first to install all tools.

```bash
mise run ci    # Run all ci:* tasks
mise run fmt   # Run all fmt:* tasks
mise run lint  # Run all lint:* tasks
mise run test  # Run all test:* tasks
```

## Tools

All tools are managed by mise. Run `mise install` to install them.

| Tool           | Purpose                                   |
| -------------- | ----------------------------------------- |
| uv             | Python package manager                    |
| dprint         | Code formatter                            |
| prek           | Pre-commit hook runner                    |
| shfmt          | Shell script formatter                    |
| actionlint     | GitHub Actions linter                     |
| zizmor         | GitHub Actions security linter            |
| shellcheck     | Shell script linter                       |
| ghalint        | GitHub Actions linter                     |
| pinact         | Pin GitHub Actions versions to SHAs       |
| rust           | Rust toolchain                            |
| cargo-binstall | Prebuilt binary installer for cargo tools |
| cargo-nextest  | Fast Rust test runner                     |
| cargo-deny     | Dependency license and advisory checker   |
| cargo-audit    | Security advisory checker for Rust        |
| cargo-mutants  | Mutation testing for Rust                 |
| node           | Node.js runtime                           |
| pnpm           | Node.js package manager                   |

# CLAUDE.md

## Purpose

Rust のデスクトップ GUI フレームワークで同じ「AI チャットアプリ」を実装し、
開発体験・性能・配布サイズを比較する検証用 monorepo。
比較対象は Tauri v2 / egui (eframe) / iced / Slint / Dioxus (desktop) の 5 つ。
プロダクト化は目的とせず、同一仕様で公平に比べられることを最優先する。

## 共通仕様（全アプリで揃える）

- 会話一覧（サイドバー）＋チャット画面＋入力欄の 2 ペイン構成
- LLM API へのストリーミング応答（トークン単位で逐次描画、途中キャンセル可）
- Markdown 表示（最低限: 見出し・リスト・コードブロック）
- 会話履歴のローカル永続化（JSON、OS 標準のデータディレクトリ）
- 設定画面: API キー・モデル・system prompt
- API キーは OS キーチェーン（`keyring` crate）に保存し、平文でファイルに書かない

## Architecture

```
crates/chat-core/        # UI 非依存のロジック（全アプリ共通）
apps/tauri/              # Tauri v2: src-tauri/ (Rust) + フロント (TS, pnpm workspace)
apps/egui/               # eframe
apps/iced/               # iced
apps/slint/              # Slint (.slint DSL)
apps/dioxus/             # Dioxus desktop
bench/                   # 計測スクリプトと結果
docs/comparison.md       # 比較表と所感
```

### chat-core

- LLM クライアント（`reqwest` + SSE ストリーミング、`tokio`）。provider は trait で抽象化し、
  Anthropic Messages API を最初に実装。テスト用にモック provider を用意する
- 会話・メッセージのドメインモデル、永続化（`serde_json`）、設定管理
- ストリームは `tokio::sync::mpsc` 等の UI 非依存なチャネルで公開し、
  各 UI 側のイベントループ（egui の repaint、iced の Subscription、Tauri の event 等）へ橋渡しする
- UI アプリはこの crate を使うだけにし、フレームワーク固有のロジックを core に入れない

### 比較観点（docs/comparison.md）

計測値:

- リリースビルドのバイナリ / インストーラサイズ
- 起動時間（コールド / ウォーム）、アイドル時と長い会話（1,000 メッセージ）表示時のメモリ
- ストリーミング中の CPU 使用率・描画のカクつき
- clean build / incremental build 時間

定性評価:

- 状態管理とストリーミング実装のしやすさ、コード量（`tokei`）
- テキスト入力・IME（日本語入力）・選択/コピー・スクロールの品質
- Markdown / コードブロック表示の手間、テーマ・見た目の自由度
- アクセシビリティ、クロスプラットフォーム（macOS / Windows / Linux）、配布のしやすさ

計測はモック provider（固定レスポンスを一定速度でストリーム）を使い、ネットワーク差を排除する。

## Notes

- テンプレートからの初期移行（`crates/chat-core` へのリネーム、workspace members、`apps/tauri` の pnpm workspace 化）は完了済み
- 全アプリが 1 つの Cargo workspace / lockfile を共有するため、`webkit2gtk-sys` は 1 バージョンしか共存できない。
  dioxus-desktop が要求する wry に合わせて Tauri のバージョンを固定している（`apps/tauri/src-tauri/Cargo.toml` 参照）
- 1 フレームワークずつ実装し、同じ仕様を満たしたら比較表を更新する。
  実装順の目安: egui → Tauri → iced → Slint → Dioxus
- Linux の CI では Tauri / 各 GUI crate のシステム依存（webkit2gtk, xkbcommon 等）のインストールが必要（`scripts/install-linux-deps.sh`）
