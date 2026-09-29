# mcp-app-viewer

[MCP Apps](https://github.com/modelcontextprotocol/ext-apps)（SEP-1865, spec `2026-01-26`）の UI リソース（`text/html;profile=mcp-app`）を
ブラウザで表示・デバッグするための Web ホスト。MCP サーバには接続しない。ツール入力・ツール結果・`tools/call` の応答は画面上の JSON で与える。

```bash
pnpm --filter mcp-app-viewer dev   # http://localhost:5180/ を開く
```

起動するとサンプル（`crates/chat-core/assets/dice-app.html`。SDK なしで postMessage を直接使うサイコロアプリ）が描画される。
自分の View を表示するには、HTML を貼るかファイルを選んで **Render** を押す。`resources/read` の結果 JSON（`contents[0]` の `text` / `blob` と `_meta.ui`）も貼れる。

## 仕様との対応

| 項目           | 実装                                                                                                                                                                                                                                                     |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Sandbox proxy  | ホストと別オリジンの `sandbox.html` を iframe で読み込み、`sandbox-proxy-ready` → `sandbox-resource-ready` で HTML を渡す。`sandbox-*` 以外は中継する                                                                                                    |
| CSP            | `_meta.ui.csp` から組み立てる（未指定なら仕様の制限的デフォルト）。`<meta>` として doctype の直後に挿入する                                                                                                                                              |
| Permissions    | `_meta.ui.permissions` を iframe の `allow` 属性に変換する                                                                                                                                                                                               |
| ライフサイクル | `ui/initialize` に hostContext / hostCapabilities を返し、`initialized` の後に `tool-input` と `tool-result` を送る。`tool-cancelled` と `ui/resource-teardown` も送れる                                                                                 |
| View → Host    | `tools/call`（モック応答。`visibility` に `app` がないツールは拒否）、`resources/read`、`ui/open-link`（確認ダイアログあり）、`ui/message`、`ui/update-model-context`、`ui/request-display-mode`（inline / fullscreen）、`ping`、`notifications/message` |
| Host → View    | テーマ・コンテナ幅・表示モードの変更を `ui/notifications/host-context-changed` で通知する。`size-changed` を受けて iframe の高さを追従させる（最大 800px）                                                                                               |
| テーマ         | `hostContext.styles.variables` に `light-dark()` を使った CSS 変数を渡す                                                                                                                                                                                 |
| 監査           | すべての JSON-RPC メッセージを方向付きでログに出す。`ui/message` などは Host inbox に表示する                                                                                                                                                            |

## Sandbox のオリジン

仕様では、Web ホストは View を別オリジンの sandbox proxy 経由で表示しなければならない。
dev サーバは `127.0.0.1` に bind する。ホストを `localhost`、sandbox を `127.0.0.1` として、1 つのサーバで 2 つのオリジンを賄う（逆でもよい）。
別のホストに配置するときは、同じ `dist/` を置いたうえで `?sandbox=https://sandbox.example.com` のように sandbox のオリジンを指定する。
