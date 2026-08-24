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

# Backend Cargo test freshness gate: the stamp records a content digest of the
# backend crate's build inputs so `make test` never reuses a stale test binary.
BACKEND_DIR := $(dir $(BACKEND_MANIFEST))
BACKEND_TEST_STAMP := $(BACKEND_DIR)target/.emuchef-cargo-test-source.sha256
BACKEND_TEST_PENDING := $(BACKEND_TEST_STAMP).pending

.PHONY: help install ensure-deps build test device-qualification-check cargo-test-freshness-check backend-test-fresh emuchef-app config-editor dev

help:
	@printf '%s\n' \
		'Available targets:' \
		'  help          Show this help message' \
		'  install       Reinstall frontend dependencies from lockfiles' \
		'  ensure-deps   Install missing or stale frontend dependencies' \
		'  build         Build the Rust backend and both frontend applications' \
		'  test          Run Rust, application, security, typecheck, and lint tests' \
		'  device-qualification-check    Validate device qualification definitions, evidence, and matrix' \
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
	cargo test --manifest-path $(BACKEND_MANIFEST)
	cargo test --manifest-path $(EMUCHEF_TAURI_MANIFEST)
	npm --prefix $(EMUCHEF_APP_PREFIX) run test
	npm --prefix $(EMUCHEF_APP_PREFIX) run test:security
	npm --prefix $(EMUCHEF_APP_PREFIX) run typecheck
	npm --prefix $(EMUCHEF_APP_PREFIX) run lint
	npm --prefix $(CONFIG_EDITOR_PREFIX) run check:rust-runtime
	npm --prefix $(CONFIG_EDITOR_PREFIX) run typecheck
	npm --prefix $(CONFIG_EDITOR_PREFIX) run lint

device-qualification-check:
	node --test tools/device-qualification.test.mjs
	node tools/device-qualification.mjs --check

cargo-test-freshness-check:
	node --test tools/cargo-test-freshness.test.mjs

backend-test-fresh:
	@node tools/cargo-test-freshness.mjs $(BACKEND_DIR) $(BACKEND_TEST_STAMP)
	@if [ -f $(BACKEND_TEST_PENDING) ]; then \
		echo 'backend-test-fresh: backend source changed; cleaning emuchef-rust-backend test artifacts'; \
		cargo clean --manifest-path $(BACKEND_MANIFEST) -p emuchef-rust-backend; \
		rm -f $(BACKEND_TEST_PENDING); \
	fi

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
