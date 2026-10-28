.DEFAULT_GOAL := help
SHELL := /bin/sh

EMUCHEF_APP_PREFIX := apps/emuchef-app
CONFIG_EDITOR_PREFIX := apps/config-editor

EMUCHEF_APP_DEPS_STAMP := $(EMUCHEF_APP_PREFIX)/node_modules/.emuchef-deps-stamp
CONFIG_EDITOR_DEPS_STAMP := $(CONFIG_EDITOR_PREFIX)/node_modules/.emuchef-deps-stamp

# Rust workspace boundary: the standalone backend workspace.
BACKEND_MANIFEST := crates/emuchef-rust-backend/Cargo.toml
# Rust workspace boundary: the EmuChef Tauri application's workspace.
EMUCHEF_TAURI_MANIFEST := apps/emuchef-app/src-tauri/Cargo.toml
# Rust workspace boundary: the Config Editor Tauri application's workspace.
CONFIG_EDITOR_TAURI_MANIFEST := apps/config-editor/src-tauri/Cargo.toml

# Backend Cargo test freshness gate: the stamp records a content digest of the
# backend crate's build inputs so `make test` never reuses a stale test binary.
BACKEND_DIR := $(dir $(BACKEND_MANIFEST))
BACKEND_TEST_STAMP := $(BACKEND_DIR)target/.emuchef-cargo-test-source.sha256
BACKEND_TEST_PENDING := $(BACKEND_TEST_STAMP).pending

# Cleanup uses an explicit worktree-local allowlist. The helper is resolved
# beside this Makefile so fixture tests can invoke this exact entrypoint from a
# disposable Git checkout without running cleanup against the developer tree.
CLEANUP_SCRIPT := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))scripts/worktree-cleanup.mjs

.PHONY: help install ensure-deps build test clean clean-preview clean-deep device-qualification-check cargo-test-freshness-check backend-test-fresh emuchef-app config-editor dev qualify-armsx1-emulator

help:
	@printf '%s\n' \
		'Available targets:' \
		'  help          Show this help message' \
		'  install       Reinstall frontend dependencies from lockfiles' \
		'  ensure-deps   Install missing or stale frontend dependencies' \
		'  build         Build the Rust backend and both frontend applications' \
		'  test          Run Rust, application, security, typecheck, and lint tests' \
		'  clean-preview Preview recognized Cargo/Tauri cleanup; no changes; idle only' \
		'  clean         Remove known intermediates; preserve bundles/runtime/tracked/unknown; idle only' \
		'  clean-deep    Opt-in warning: remove deps/dist/sidecars/metadata; preserve tracked/unknown; idle only' \
		'  device-qualification-check    Validate device qualification definitions, evidence, and matrix' \
		'  qualify-armsx1-emulator      Opt-in live qualification; requires exact dedicated AVD identity and operator attestation' \
		'  emuchef-app   Launch the EmuChef app in development mode' \
		'  config-editor Launch the Config Editor app in development mode' \
		'  dev           Launch both applications in development mode'

install:
	npm --prefix $(EMUCHEF_APP_PREFIX) ci
	@touch $(EMUCHEF_APP_DEPS_STAMP)
	npm --prefix $(CONFIG_EDITOR_PREFIX) ci
	@touch $(CONFIG_EDITOR_DEPS_STAMP)

ensure-deps: $(EMUCHEF_APP_DEPS_STAMP) $(CONFIG_EDITOR_DEPS_STAMP)

$(EMUCHEF_APP_DEPS_STAMP): $(EMUCHEF_APP_PREFIX)/package.json $(EMUCHEF_APP_PREFIX)/package-lock.json
	npm --prefix $(EMUCHEF_APP_PREFIX) ci
	@touch $@

$(CONFIG_EDITOR_DEPS_STAMP): $(CONFIG_EDITOR_PREFIX)/package.json $(CONFIG_EDITOR_PREFIX)/package-lock.json
	npm --prefix $(CONFIG_EDITOR_PREFIX) ci
	@touch $@

build: ensure-deps
	cargo build --manifest-path $(BACKEND_MANIFEST)
	npm --prefix $(EMUCHEF_APP_PREFIX) run build
	npm --prefix $(CONFIG_EDITOR_PREFIX) run build

test: ensure-deps device-qualification-check cargo-test-freshness-check backend-test-fresh
	node --test scripts/worktree-cleanup*.test.mjs
	node --test tools/qualify-armsx1-emulator.test.mjs
	cargo test --manifest-path $(BACKEND_MANIFEST)
	cargo test --manifest-path $(EMUCHEF_TAURI_MANIFEST)
	npm --prefix $(EMUCHEF_APP_PREFIX) run test
	npm --prefix $(EMUCHEF_APP_PREFIX) run test:security
	npm --prefix $(EMUCHEF_APP_PREFIX) run typecheck
	npm --prefix $(EMUCHEF_APP_PREFIX) run lint
	npm --prefix $(CONFIG_EDITOR_PREFIX) run check:rust-runtime
	npm --prefix $(CONFIG_EDITOR_PREFIX) run typecheck
	npm --prefix $(CONFIG_EDITOR_PREFIX) run lint

# Explicit opt-in only. The runner independently validates an exclusively dedicated AVD.
qualify-armsx1-emulator:
	node --test tools/qualify-armsx1-emulator.test.mjs
	node tools/qualify-armsx1-emulator.mjs

device-qualification-check:
	node --test tools/device-qualification.test.mjs
	node tools/device-qualification.mjs --check

cargo-test-freshness-check:
	node --test tools/cargo-test-freshness.test.mjs

backend-test-fresh:
	@node tools/cargo-test-freshness.mjs $(BACKEND_DIR) $(BACKEND_TEST_STAMP)
	@if [ -f $(BACKEND_TEST_PENDING) ]; then \
		echo 'backend-test-fresh: backend source changed; cleaning emuchef-rust-backend test artifacts'; \
		cargo clean --manifest-path $(BACKEND_MANIFEST) -p emuchef-rust-backend || exit $$?; \
		rm -f $(BACKEND_TEST_PENDING); \
	fi

# Cleanup validates this worktree, scans explicit artifact allowlists, and
# preserves tracked and unknown files. Clean is not guaranteed safe during
# concurrent builds; run it while build/test commands are idle.
clean-preview:
	@node "$(CLEANUP_SCRIPT)" preview

clean:
	@node "$(CLEANUP_SCRIPT)" default

clean-deep:
	@node "$(CLEANUP_SCRIPT)" deep

# Ordinary app development is simulation-only; real execution requires its separate guarded command.
emuchef-app: ensure-deps
	npm --prefix $(EMUCHEF_APP_PREFIX) run tauri:dev

config-editor: ensure-deps
	npm --prefix $(CONFIG_EDITOR_PREFIX) run tauri -- dev

dev: ensure-deps
	@set -eu; \
	cleaned_up=0; \
	cleanup() { \
		status="$$1"; \
		signal="$${2:-TERM}"; \
		if [ "$$cleaned_up" -eq 1 ]; then exit "$$status"; fi; \
		cleaned_up=1; \
		trap - INT TERM EXIT; \
		if [ -n "$${pids:-}" ]; then \
			for pid in $$pids; do kill "-$$signal" "-$$pid" 2>/dev/null || true; done; \
			wait $$pids 2>/dev/null || true; \
		fi; \
		exit "$$status"; \
	}; \
	trap 'cleanup "$$?" TERM' EXIT; \
	pending_signal=; \
	record_signal() { pending_signal="$$1"; }; \
	trap 'record_signal INT' INT; \
	trap 'record_signal TERM' TERM; \
	if ! command -v perl >/dev/null 2>&1 || ! perl -MPOSIX -e 'POSIX::setsid() or exit 1' >/dev/null 2>&1; then \
		echo 'dev: Perl with POSIX::setsid is required to launch app sessions' >&2; \
		exit 1; \
	fi; \
	launch_in_session() { perl -MPOSIX -e 'POSIX::setsid() or die "setsid: $$!\\n"; exec @ARGV or die "exec: $$!\\n";' -- "$$@"; }; \
	launch_in_session npm --prefix $(EMUCHEF_APP_PREFIX) run tauri:dev & emuchef_pid="$$!"; \
	pids="$$emuchef_pid"; \
	launch_in_session npm --prefix $(CONFIG_EDITOR_PREFIX) run tauri -- dev & config_pid="$$!"; \
	pids="$$pids $$config_pid"; \
	trap 'cleanup 130 INT' INT; \
	trap 'cleanup 143 TERM' TERM; \
	case "$$pending_signal" in \
		INT) cleanup 130 INT;; \
		TERM) cleanup 143 TERM;; \
	esac; \
	while :; do \
		if ! kill -0 "$$emuchef_pid" 2>/dev/null; then \
			if wait "$$emuchef_pid"; then child_status=0; else child_status="$$?"; fi; \
			[ "$$child_status" -eq 0 ] && child_status=1; \
			cleanup "$$child_status" TERM; \
		fi; \
		if ! kill -0 "$$config_pid" 2>/dev/null; then \
			if wait "$$config_pid"; then child_status=0; else child_status="$$?"; fi; \
			[ "$$child_status" -eq 0 ] && child_status=1; \
			cleanup "$$child_status" TERM; \
		fi; \
		sleep 1; \
	done
