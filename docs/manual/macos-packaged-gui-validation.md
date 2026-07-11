# macOS Packaged Config Editor Validation

This runbook records release-bundle, packaged-runtime, sidecar-protocol, and
interactive editor evidence for one exact EmuChef commit. Automated checks do
not establish visual or interactive success. GUI-triggered planning and apply
are not product features and are not acceptance criteria for this runbook.

## A. Preconditions

1. Use a supported macOS host. Record the exact version; the bundle's
   `LSMinimumSystemVersion` is evidence, not a substitute for testing the
   supported release host.
2. Install Xcode command-line tools, Rust, Cargo, Node, and npm.
3. Start from a clean tracked worktree. Generated ignored build outputs may be
   absent because the maintained build creates them.
4. Confirm the host architecture with `uname -m`. This runbook builds the host
   target and expects the application and sidecar architectures to match it.
5. Confirm sufficient disk space for two Rust target trees, the frontend build,
   the `.app`, the DMG, and temporary smoke-test data.
6. Local validation does not require Developer ID signing or notarization.
   Unsigned or ad-hoc signed output is expected unless release policy changes.
7. Use a disposable authored root and disposable output directory for editor,
   save, Save As, malformed-input, and network tests.
8. A safe Android device is optional and must be used only for a separately
   approved destructive apply. This runbook requires no destructive device
   operation.

## B. Record the Environment

From the repository root, create an external evidence directory and capture the
environment. Keep the directory outside the repository and redact it before
sharing:

```bash
export REPO_ROOT="$PWD"
export EVIDENCE_ROOT="$(mktemp -d /tmp/emuchef-macos-package.XXXXXX)"
git rev-parse HEAD | tee "$EVIDENCE_ROOT/commit.txt"
git status --short | tee "$EVIDENCE_ROOT/git-status.txt"
sw_vers | tee "$EVIDENCE_ROOT/sw-vers.txt"
uname -m | tee "$EVIDENCE_ROOT/architecture.txt"
rustc --version | tee "$EVIDENCE_ROOT/rustc.txt"
cargo --version | tee "$EVIDENCE_ROOT/cargo.txt"
node --version | tee "$EVIDENCE_ROOT/node.txt"
npm --version | tee "$EVIDENCE_ROOT/npm.txt"
xcode-select -p | tee "$EVIDENCE_ROOT/xcode-select.txt"
df -h "$REPO_ROOT" | tee "$EVIDENCE_ROOT/disk-space.txt"
test -z "$(git status --short)"
```

## C. Build Prerequisites

The maintained release-sidecar command is `npm run sidecar:build`. It compiles
the host-target release `emuchef` binary, copies it to Tauri's ignored
`externalBin` input name, makes it executable, and records generated metadata.
The canonical Tauri build runs that command automatically through
`beforeBuildCommand`.

```bash
cd "$REPO_ROOT/apps/config-editor"
npm ci
npm run check:rust-runtime
npm run sidecar:build
npm run check:sidecar:bundle-input
```

Do not assume `src-tauri/binaries` is populated before these commands. The
generated binary and metadata are ignored build inputs, not source files.

## D. Build and Discover the Release Bundles

Use the maintained npm/Tauri entrypoint rather than an ad hoc Cargo bundle
command:

```bash
cd "$REPO_ROOT/apps/config-editor"
npm run tauri build 2>&1 | tee "$EVIDENCE_ROOT/tauri-build.log"
```

Discover the outputs produced by this build. Do not hard-code the product name,
version, architecture, or filename:

```bash
setopt null_glob
app_candidates=(src-tauri/target/release/bundle/macos/*.app)
dmg_candidates=(src-tauri/target/release/bundle/dmg/*.dmg)
(( ${#app_candidates[@]} == 1 ))
(( ${#dmg_candidates[@]} == 1 ))
export APP_PATH="${app_candidates[1]:A}"
export DMG_PATH="${dmg_candidates[1]:A}"
printf '%s\n' "$APP_PATH" | tee "$EVIDENCE_ROOT/app-path.txt"
printf '%s\n' "$DMG_PATH" | tee "$EVIDENCE_ROOT/dmg-path.txt"
test -d "$APP_PATH"
test -f "$DMG_PATH"
```

If stale bundles produce multiple candidates, remove only the generated
`src-tauri/target/release/bundle` directory, rebuild, and repeat discovery.

## E. Static Bundle Inspection

Read the executable name from Info.plist and verify the stable sidecar name:

```bash
export MAIN_EXECUTABLE="$(plutil -extract CFBundleExecutable raw "$APP_PATH/Contents/Info.plist")"
export MAIN_PATH="$APP_PATH/Contents/MacOS/$MAIN_EXECUTABLE"
export SIDECAR_PATH="$APP_PATH/Contents/MacOS/emuchef"
test -f "$APP_PATH/Contents/Info.plist"
test -x "$MAIN_PATH"
test -x "$SIDECAR_PATH"
find "$APP_PATH" -maxdepth 4 -print | tee "$EVIDENCE_ROOT/app-tree.txt"
file "$MAIN_PATH" "$SIDECAR_PATH" | tee "$EVIDENCE_ROOT/executable-types.txt"
plutil -p "$APP_PATH/Contents/Info.plist" | tee "$EVIDENCE_ROOT/info-plist.txt"
otool -L "$MAIN_PATH" | tee "$EVIDENCE_ROOT/main-libraries.txt"
otool -L "$SIDECAR_PATH" | tee "$EVIDENCE_ROOT/sidecar-libraries.txt"
codesign -dv --verbose=4 "$APP_PATH" 2>&1 | tee "$EVIDENCE_ROOT/codesign.txt"
xattr -l "$APP_PATH" > "$EVIDENCE_ROOT/xattrs.txt" 2>&1 || true
shasum -a 256 "$MAIN_PATH" "$SIDECAR_PATH" "$DMG_PATH" | tee "$EVIDENCE_ROOT/digests.txt"
```

The current Tauri bundle embeds compiled web assets into the main executable;
a standalone `Contents/Resources` directory is not required. `Info.plist`, the
main executable, the sidecar, and embedded production frontend markers are the
required bundle resources.

Run the maintained inspection and smoke commands after they are available in
the tested commit:

```bash
npm run check:macos-bundle -- "$APP_PATH"
npm run smoke:macos-packaged-app -- "$APP_PATH"
```

The checks must establish all of the following:

1. The `.app`, Info.plist, main executable, and sidecar exist.
2. Both executables are runnable and match the host architecture.
3. Info.plist matches the configured identifier, version, and executable.
4. No Python executable, Python framework/source, legacy runtime name, or
   shadow planner is bundled.
5. No Vite development-server URL is embedded in the release application.
6. Dynamic dependencies contain no Python runtime.
7. Signing is absent or ad hoc unless a later release policy requires more.
8. No notarization, quarantine, or Gatekeeper claim is inferred from these
   static checks.

For bounded manual confirmation, search only for forbidden terms instead of
printing complete binary strings:

```bash
find "$APP_PATH" -type f \( -iname '*python*' -o -iname '*.py' -o -iname '*.pyc' -o -iname '*legacy*' -o -iname '*shadow*' \) -print
strings -a "$MAIN_PATH" | grep -E -m 20 'http://localhost:[0-9]+|127\.0\.0\.1:[0-9]+|emuchef-python-legacy|emuchef-plan-shadow|plan_shadow' || true
strings -a "$SIDECAR_PATH" | grep -E -m 20 'emuchef-python-legacy|emuchef-plan-shadow|plan_shadow|Python\.framework' || true
```

## F. Launch the Packaged Application

First launch through Launch Services:

```bash
open "$APP_PATH"
```

Complete the first-launch checks, quit normally, launch a second time, and
confirm the app reopens without a Vite server or terminal-owned backend.

For process-level logs, quit the first instance and launch the main executable
directly:

```bash
"$MAIN_PATH" \
  > "$EVIDENCE_ROOT/direct-launch.stdout.log" \
  2> "$EVIDENCE_ROOT/direct-launch.stderr.log" &
export APP_PID=$!
sleep 5
kill -0 "$APP_PID"
ps -axo pid=,ppid=,command= | grep -F "$SIDECAR_PATH --sidecar" \
  | tee "$EVIDENCE_ROOT/sidecar-process.txt"
```

In the app, check sidecar status, ping behavior, backend restart, recovery, and
clean shutdown. Then quit normally and verify neither executable remains:

```bash
ps -axo pid=,ppid=,command= | grep -F "$APP_PATH/Contents/MacOS/" || true
```

Process observation is supporting evidence. Direct JSONL hello/ping against the
exact bundled sidecar and visible application behavior remain the stronger
protocol and GUI evidence; no parent-child instrumentation is added to the
product.

## G. Interactive Editor Workflow

Use a disposable copy of an existing authored recipe. Record pass, fail, or
blocked for every item:

1. The packaged app opens without a Vite development server.
2. The selected authored root is displayed correctly.
3. Setting, changing, and clearing the authored root behave correctly.
4. An existing recipe opens from the disposable authored root.
5. Validation completes and diagnostics are understandable.
6. The recipe overview displays and its editable metadata can be changed.
7. Inputs can be added or edited.
8. Artifacts and cache settings can be added or edited.
9. Artifact groups and memberships can be edited.
10. A step can be added and edited.
11. Step dependencies can be edited.
12. Structured step parameters can be edited.
13. Constraints can be edited.
14. `skipIf` can be edited.
15. Verification conditions can be edited.
16. Advanced JSON editors work for supported fallback shapes.
17. Undo restores the preceding state.
18. Redo reapplies the undone state.
19. Save writes the disposable source file.
20. Save As writes a separate file.
21. Saved and Save As content reopen with the expected structure.
22. Opening another file with dirty content shows the discard prompt.
23. Closing the window with dirty content shows the close prompt.
24. An in-flight operation prevents unsafe open, close, or restart actions.
25. Backend restart is available when safe.
26. The UI clearly marks the old document session invalid after restart.
27. Reopening from disk restores a usable document session.
28. Application relaunch preserves saved filesystem content.
29. A second launch starts and recovers the packaged sidecar normally.

The Config Editor does not generate execution plans or invoke apply. Do not add
or simulate those interfaces during this validation. GUI-triggered plan/apply
remains a future product capability.

## H. Packaged-Runtime HTTP and TLS Workflow

Run the deterministic smoke against the exact `Contents/MacOS/emuchef`
executable:

```bash
npm run smoke:packaged-runtime-network -- "$APP_PATH"
```

The smoke uses disposable authored data and no real device. Record these three
evidence categories separately:

1. Packaged-runtime local HTTP success:
   - validate and plan succeed;
   - cold dry-run apply downloads once;
   - warm dry-run apply reuses the cache without another request;
   - offline warm-cache dry-run succeeds after the server stops;
   - cache bytes are unchanged and no `*.partial` file remains;
   - an HTTP failure is typed, redacted, and leaves no partial publication.
2. Packaged-runtime local untrusted-HTTPS behavior:
   - a self-signed local TLS origin is rejected;
   - the exposed failure is typed and redacted;
   - no final or partial artifact is published;
   - no custom CA or trust-store override is used.
3. Trusted-HTTPS success:
   - cite
     `docs/release/evidence/real-device-retroarch-2026-07-11.md`, which records
     successful trusted HTTP(S) resolution on
     `5dca50603cf3a4831867c229157a94906151cbb7`.

This is packaged-runtime evidence, not packaged-GUI HTTP evidence. The current
editor sidecar protocol has no planning or apply request.

## I. Failure and Recovery Matrix

Use a disposable copy of the `.app` whenever a test alters bundle contents.

| Case | Expected behavior |
| --- | --- |
| Sidecar missing | Startup or the first sidecar request fails clearly; the app does not fall back to a development path or Python. |
| Sidecar not executable | Startup or the first request reports an executable/launch failure; no fallback is attempted. |
| Sidecar exits during a request | The request fails, status reports the stopped process, and restart creates a new session rather than pretending the old document session survived. |
| Malformed authored YAML | Open or validation returns a structured load/validation error without crashing the app. |
| Invalid execution plan | Direct packaged CLI apply rejects it before device mutation; the editor does not claim to open execution plans. |
| HTTP 404 | Packaged dry-run apply fails with `artifact_http_status`, redacts sensitive URL data, and leaves no partial publication. |
| TLS failure | The self-signed origin fails closed with typed/redacted output and no published artifact. |
| Cache write failure | A safely unwritable disposable cache fails with a cache-write error and no partial publication. Skip if it cannot be reproduced without changing protected host state. |
| Dirty document during backend restart | The UI prevents or clearly confirms the unsafe action and does not silently discard edits. |
| Reopen after abnormal termination | Saved content reopens; unsaved process-local edits are not falsely reported as recovered. |

Example disposable bundle preparation:

```bash
export FAILURE_ROOT="$(mktemp -d /tmp/emuchef-macos-failure.XXXXXX)"
ditto "$APP_PATH" "$FAILURE_ROOT/Failure Test.app"
export FAILURE_APP="$FAILURE_ROOT/Failure Test.app"
```

Never modify the primary evidence bundle in place.

## J. Evidence Record

| Field | Value |
| --- | --- |
| Date | |
| Operator | |
| Commit SHA | |
| macOS version | |
| Architecture | |
| App bundle path | |
| DMG path and digest | |
| Main executable digest and architecture | |
| Sidecar digest and architecture | |
| Build command | |
| Static bundle inspection | |
| Signing state | |
| First and second launch | |
| Direct launch logs | |
| Sidecar JSONL/status/ping | |
| Editor workflow | |
| Packaged-runtime local HTTP | |
| Packaged-runtime untrusted HTTPS | |
| Trusted-HTTPS evidence reference | |
| Cache and partial-file result | |
| Failure tests | |
| Screenshots location | |
| Logs location | |
| Deviations | |
| Blockers | |
| Final disposition | |

Keep screenshots, raw logs, paths containing usernames, and other host-specific
evidence outside the repository unless separately sanitized.

## K. Pass Criteria

A packaged-GUI milestone pass requires all of the following:

1. The exact commit and environment are recorded.
2. The release build succeeds and its actual `.app` and DMG outputs are found.
3. The app, sidecar, Info.plist, architectures, dependencies, and signing state
   pass static inspection.
4. The release bundle contains no Python runtime, shadow planner, legacy
   sidecar, or Vite development-server dependency.
5. The main application launches and the exact bundled sidecar passes direct
   JSONL hello/ping and process-level startup checks.
6. The core document workflow, Save, Save As, prompts, in-flight guards,
   backend restart, recovery, and relaunch pass interactively.
7. Packaged-runtime local HTTP cold/warm/offline behavior passes with identical
   cache bytes and no partial files.
8. Packaged-runtime self-signed HTTPS fails closed with typed/redacted output.
9. The trusted-HTTPS real-device evidence is linked without claiming a new
   packaged local trusted-HTTPS run.
10. No unexplained crash or protocol failure remains.

Signing, notarization, updater support, Windows/Linux packaging, and
GUI-triggered plan/apply are outside this milestone and must remain explicitly
classified rather than silently treated as passed.

