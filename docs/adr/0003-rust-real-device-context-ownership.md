# ADR 0003: Rust Real-Device Context Ownership

## Status

Accepted

## Decision

Rust owns live ADB probing, detected-device interpretation, profile mismatch
warnings, and explicit device-context precedence for planning. Device probing
uses the exact supplied ADB path and optional serial without shell execution.
