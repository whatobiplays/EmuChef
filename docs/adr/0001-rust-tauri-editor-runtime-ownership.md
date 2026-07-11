# ADR 0001: Rust Tauri Editor Runtime Ownership

## Status

Accepted

## Decision

The Tauri config editor uses the Rust `emuchef --sidecar` process as its only
backend. The frontend and Tauri shell must not add Python execution, fallback,
or backend selection. Rust owns editor document sessions and the JSONL protocol.
