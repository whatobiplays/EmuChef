# Device Storage Preflight Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a safe, resumable Node CLI that prepares an Android device for bounded storage-constrained qualification and removes only its owned allocation.

**Architecture:** `tools/device-storage-preflight.mjs` contains immutable profiles, pure parsers/planners, an injectable device-storage workflow, and the real ADB adapter. `tools/device-storage-preflight.test.mjs` tests pure behavior and full prepare/cleanup flows through a fake device without touching ADB.

**Tech Stack:** Node.js ES modules, `node:test`, `node:assert/strict`, `node:child_process`, Android Platform-Tools.

## Global Constraints

- Do not change the production executor, storage classifier, evidence schema, physical harness semantics, deadlines, or cleanup ownership.
- Do not allocate host-side multi-gigabyte payloads.
- Mutations are limited to `/sdcard/Download/EmuChefStoragePreflight/<profile>`.
- Require exactly one selected authorized ADB device and the committed fixture package.
- Preparation requires `--yes`; status and dry-run never mutate.
- Cleanup requires the exact ownership marker and rejects unknown directory entries.
- Do not stage, commit, or push.

---

### Task 1: Pure storage and ownership contract

**Files:**
- Create: `tools/device-storage-preflight.test.mjs`
- Create: `tools/device-storage-preflight.mjs`

**Interfaces:**
- Produces: `STORAGE_PROFILES`, `profileFor`, `parseDfKib`, `parseAdbDevices`, `validateSelectedDevice`, `validateProfile`, `planNextChunkKib`, `nextChunkName`, `validateOwnedEntries`, and `validateObservedConsumption`.

- [ ] **Step 1: Write failing tests** for header-aware `df`, exact inventory selection, profile constants, chunk tiers, target readiness, ownership entries, and observed storage deltas.
- [ ] **Step 2: Run** `rtk node --test tools/device-storage-preflight.test.mjs` and confirm failure because the module or exports are absent.
- [ ] **Step 3: Implement the minimal pure functions** with strict integer, path, serial, marker, and filename validation.
- [ ] **Step 4: Re-run** `rtk node --test tools/device-storage-preflight.test.mjs` and confirm the pure tests pass.

### Task 2: Injectable prepare and cleanup workflows

**Files:**
- Modify: `tools/device-storage-preflight.test.mjs`
- Modify: `tools/device-storage-preflight.mjs`

**Interfaces:**
- Consumes: the Task 1 profile and planning functions.
- Produces: `inspectStorage`, `prepareStorage`, and `cleanupStorage`, accepting a device adapter with inventory, package, `df`, path, marker, chunk-write, sync, and removal methods.

- [ ] **Step 1: Write failing fake-device tests** for dry-run, fresh preparation, resume, filesystem mismatch, below-minimum refusal, unexpected consumption, owned cleanup, and unowned cleanup rejection.
- [ ] **Step 2: Run** the focused Node test and confirm the new cases fail for missing workflows.
- [ ] **Step 3: Implement synchronous workflows** that re-inspect before every chunk, stop inside the profile window, retain partial owned chunks on failure, and verify cleanup absence.
- [ ] **Step 4: Re-run** the focused Node test and confirm all fake-device cases pass.

### Task 3: Real ADB adapter and CLI

**Files:**
- Modify: `tools/device-storage-preflight.mjs`
- Modify: `tools/device-storage-preflight.test.mjs`

**Interfaces:**
- Produces: `AdbStorageDevice` and `runCli` with `status`, `prepare`, and `cleanup` commands.

- [ ] **Step 1: Write failing source/CLI tests** for portable ES-module entry detection, confirmation gating, dry-run output, recovery commands, and strict profile lookup.
- [ ] **Step 2: Run** the focused Node test and confirm failure for missing CLI behavior.
- [ ] **Step 3: Implement the ADB adapter** with `execFileSync`, fixed profile paths, direct device-local `dd`, explicit `sync`, bounded output, and shell quoting only for validated constants.
- [ ] **Step 4: Implement CLI output** that reports free space, readiness, each observed chunk delta, the exact harness block, and the exact cleanup command.
- [ ] **Step 5: Re-run** the focused Node test.

### Task 4: Runbook and current-state documentation

**Files:**
- Modify: `docs/manual/phase-6d6-physical-interruption-qualification.md`
- Modify: `docs/product/phase-6d6-physical-interruption-qualification.md`
- Modify: `CONTEXT.md`

- [ ] **Step 1: Document** status, dry-run, confirmed preparation, low-storage repetitions, retained preflight allocation, and final cleanup.
- [ ] **Step 2: State explicitly** that the utility is operator preflight support, not physical evidence and not a production execution feature.
- [ ] **Step 3: Add** `rtk node --test tools/device-storage-preflight.test.mjs` to the documented host-only verification commands.

### Task 5: Verification

- [ ] **Step 1: Run** `rtk node --test tools/device-storage-preflight.test.mjs`.
- [ ] **Step 2: Run** `rtk node --test tools/phase-6d6-evidence.test.mjs`.
- [ ] **Step 3: Run** `rtk node --test tools/phase-6d6-evidence-regression.test.mjs`.
- [ ] **Step 4: Run** `rtk node tools/phase-6d6-evidence.mjs`.
- [ ] **Step 5: Run** `rtk git diff --check` and `rtk git status --short`.
