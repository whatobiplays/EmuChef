# Makefile Development Workflow Design

## Purpose

Add a root-level `Makefile` that provides stable, discoverable commands for
building the repository, running the complete automated validation suite, and
launching both Tauri applications during development.

## Scope

The Makefile is an orchestration layer only. Existing Cargo manifests and npm
scripts remain authoritative for build and test behavior. The Makefile must
not introduce a new runtime, alter application architecture, or enable the
default-off `real-execution` Cargo feature.

## Targets

- `make build` builds the Rust backend workspace and both application
  frontends using their existing build commands.
- `make test` runs Rust tests for the backend and end-user Tauri workspace,
  then runs the end-user app's test, security, typecheck, and lint commands,
  followed by the Config Editor's full `check:rust-runtime`, typecheck, and
  lint commands.
- `make emuchef-app` launches the end-user app using its existing simulation-
  only `tauri:dev` script.
- `make config-editor` launches the Config Editor using its existing Tauri
  development command.
- `make dev` launches both application targets concurrently and terminates
  both child processes when the combined development session is interrupted.
- `make help` lists the supported developer-facing targets.

All targets are declared phony. Commands use repository-relative paths and
existing `npm --prefix` or `--manifest-path` forms so they work from the
repository root.

## Failure and process behavior

Build and test commands stop on the first failure and return a non-zero exit
status. The combined development target must forward interrupt/termination
signals to both application processes and return their session status without
leaving background app processes behind.

## Documentation

`CONTEXT.md` will describe the Makefile targets as current developer commands,
including the distinction between simulation-only development and the
separate guarded real-execution command.

## Verification

Verification will inspect the Makefile's command definitions, run `make help`,
run the build and test targets, and check that the development target starts
both apps without enabling `real-execution`. Full manual GUI, accessibility,
packaging, signing, and release qualification are outside this change.
