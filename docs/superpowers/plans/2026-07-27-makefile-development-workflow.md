# Makefile Development Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a root Makefile that builds the repository, runs the complete automated validation suite, and launches both Tauri applications together for development.

**Architecture:** Keep the Makefile as a thin root-level orchestration layer. Cargo manifests and package scripts remain authoritative; Make invokes them with explicit repository paths. The combined development target uses a small POSIX shell process supervisor to start both existing app commands and clean them up together.

**Tech Stack:** GNU Make-compatible Makefile syntax, POSIX shell, Cargo, npm, Rust, React, and Tauri.

## Global Constraints

- Do not substantially change application architecture.
- Do not enable the default-off `real-execution` Cargo feature.
- Build and test commands must stop on failure with a non-zero status.
- Preserve unrelated worktree changes.
- Keep `CONTEXT.md` as standalone current facts, not a change log.
- Automated verification must not imply manual GUI, accessibility, packaging, signing, or release qualification.

---

### Task 1: Add the root Makefile

**Files:**
- Create: `Makefile`

**Interfaces:**
- Produces `help`, `build`, `test`, `emuchef-app`, `config-editor`, and `dev` targets.
- `dev` launches `npm --prefix apps/emuchef-app run tauri:dev` and `npm --prefix apps/config-editor run tauri` concurrently.

- [ ] **Step 1: Define target names and shared command variables**

Declare `.PHONY` for all public targets, define the backend manifest paths and
app prefixes once, and make `help` the default target. Keep paths explicit so
the Makefile is runnable only from the repository root and does not depend on
an ambient working directory.

- [ ] **Step 2: Add the build target**

`build` must run, in order:

```make
cargo build --manifest-path crates/emuchef-rust-backend/Cargo.toml
npm --prefix apps/emuchef-app run build
npm --prefix apps/config-editor run build
```

Do not call a release packaging target or pass `--features real-execution`.

- [ ] **Step 3: Add the full test target**

`test` must run these existing commands in order:

```make
cargo test --manifest-path crates/emuchef-rust-backend/Cargo.toml
cargo test --manifest-path apps/emuchef-app/src-tauri/Cargo.toml
npm --prefix apps/emuchef-app run test
npm --prefix apps/emuchef-app run test:security
npm --prefix apps/emuchef-app run typecheck
npm --prefix apps/emuchef-app run lint
npm --prefix apps/config-editor run check:rust-runtime
npm --prefix apps/config-editor run typecheck
npm --prefix apps/config-editor run lint
```

This relies on each package's existing test scripts for its package-specific
logic and runtime checks.

- [ ] **Step 4: Add individual app launch targets**

Use the existing development commands without changing their environment:

```make
emuchef-app:
	 npm --prefix apps/emuchef-app run tauri:dev

config-editor:
	 npm --prefix apps/config-editor run tauri
```

The end-user app target must remain simulation-only by using `tauri:dev`, not
`tauri:dev:real`.

- [ ] **Step 5: Add the combined development supervisor**

Start both launch commands in the background, retain both PIDs, install an
`INT`/`TERM` trap that sends those signals to both children and waits for them,
and return a failure status if either child exits unexpectedly. Ensure cleanup
runs on normal shell exit as well, so interrupting `make dev` does not leave
either Tauri process behind.

- [ ] **Step 6: Add help output and comments**

Make `help` print each public target and its purpose. Add concise comments for
the two Rust workspace boundaries and the simulation-only development rule.

- [ ] **Step 7: Verify the Makefile syntax and help target**

Run:

```bash
make -n build
make -n test
make help
```

Expected: the dry-run output contains every command listed above, contains no
`real-execution` feature flag, and `make help` exits successfully with all
public targets listed.

- [ ] **Step 8: Commit the Makefile**

```bash
git add Makefile
git commit -m "build: add repository Makefile workflows"
```

### Task 2: Document and verify the developer workflow

**Files:**
- Modify: `CONTEXT.md` in the current developer-command section

**Interfaces:**
- Documents the root Makefile targets as current repository facts.
- Does not rewrite unrelated historical or product requirements.

- [ ] **Step 1: Add current Makefile command documentation**

Document that `make build` builds both Rust/frontend codebases, `make test`
runs the full automated suite, `make emuchef-app` and `make config-editor`
launch individual apps, and `make dev` launches both together. State that
ordinary development remains simulation-only and the separate
`tauri:dev:real` command is intentionally not used by these Makefile targets.

- [ ] **Step 2: Run the build target**

Run:

```bash
make build
```

Expected: backend and both frontend build commands complete successfully.

- [ ] **Step 3: Run the full test target**

Run:

```bash
make test
```

Expected: all listed Rust, app test, security, typecheck, and lint commands
complete successfully. Record any pre-existing warnings or environment-only
limitations without treating them as passing evidence.

- [ ] **Step 4: Verify combined launch behavior**

Run `make dev` long enough to confirm both existing Tauri development commands
start, then interrupt it with Ctrl-C. Verify that the Make process exits and no
child development processes remain. Do not treat this as packaged GUI,
accessibility, signing, or release qualification.

- [ ] **Step 5: Inspect the final diff and status**

Run:

```bash
git diff -- Makefile CONTEXT.md
git status --short
```

Confirm only the intended files are changed and unrelated worktree changes are
preserved.

- [ ] **Step 6: Commit the documentation and verification changes**

```bash
git add CONTEXT.md
git commit -m "docs: document Makefile development commands"
```
