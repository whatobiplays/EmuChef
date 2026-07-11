# Frozen Python Reference Code

The Python packages under `src/emuchef` and `src/emuchef_editor` are frozen
reference code pending deletion. They are not product runtimes, executable
entrypoints, alternate backends, fixture generators, or compatibility oracles.

The Rust `emuchef` binary owns the CLI, planning, validation, execution,
real-ADB apply, and editor sidecar protocol. New product behavior and tests must
be implemented in Rust or the Tauri frontend as appropriate. Changes to the
retained Python code are limited to deletion work and corrections needed to
keep the reference source internally readable until it is removed.
