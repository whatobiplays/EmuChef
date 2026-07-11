# Editor Runtime

The React frontend invokes Tauri commands. Tauri forwards document operations
to one long-running `emuchef --sidecar` process over JSONL. The Rust sidecar
owns document loading, canonical YAML, diagnostics, ref indexes, command
application, undo/redo history, and saves.

Document commands use product-facing names without a transport prefix. The
`sidecar_` prefix is reserved for status, ping, and restart operations that
manage the process itself. Tauri removes transport request ids from responses
before returning protocol envelopes to the frontend.

The sidecar is in-memory and process-local. Restart invalidates document ids;
the frontend keeps stale documents read-only and offers a controlled reopen
from disk where possible.

## Content Security Policy

The production Tauri configuration contains no development URL and applies a
local-only CSP. `default-src`, scripts, styles, and fonts are limited to
`'self'`; object embedding and ancestor framing are disabled; forms remain
same-origin; images may additionally use `data:` for embedded local content.
`connect-src` permits only Tauri's `ipc:` protocol and
`http://ipc.localhost`. Artifact HTTP(S) transfers run in Rust and do not
require frontend network origins.

Development settings live in `tauri.dev.conf.json` and are selected only for
`tauri dev`. The development CSP adds the specific Vite HMR WebSocket and
`'unsafe-inline'` for development-injected styles. It does not allow
`'unsafe-eval'`, wildcard scripts, or wildcard connections, and it is not
merged into release builds.
