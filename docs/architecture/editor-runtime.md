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
