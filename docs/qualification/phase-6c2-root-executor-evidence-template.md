# Phase 6C.2 Root Executor Qualification Evidence

## 1. Run identity

- Date and operator:
- EmuChef commit:
- Platform-Tools revision:
- Fixture APK SHA-256:
- Sanitized device identity:

## 2. Device facts

- Manufacturer and model:
- Android version:
- Android API level:
- Build fingerprint:
- ABI:
- Active Android user (`0` required):
- Root implementation and version, when safely available:
- Supported `su -c` behavior:

Do not record the raw serial, root-manager secrets, credentials, or arbitrary
root command output.

## 3. Exact authority

- `EMUCHEF_RUN_REAL_ADB_TESTS=1`: yes / no
- `EMUCHEF_RUN_REAL_ADB_ROOT_TESTS=1`: yes / no
- Exact device serial selected without recording its value: yes / no
- Exact package allowlist `com.emuchef.fixture`: yes / no
- Exact committed two-prefix allowlist: yes / no
- Destructive opt-in for mutating groups: yes / no / not applicable
- Exactly one group selected per invocation: yes / no

## 4. Group outcomes

| Group | Command captured | Operation outcome | Cleanup outcome | Unsupported capability or limitation |
|---|---|---|---|---|
| Root preflight | | | not attempted | |
| Private predicates/filesystem | | | | |
| Privileged copy | | | | |
| Combined executor workflow | | | | |
| Controlled cleanup failure | | | expected failed | |

Allowed operation outcomes are `succeeded`, `preflight_failed`, and
`operation_failed`. Allowed cleanup outcomes are `not_attempted`, `succeeded`,
and `failed`.

For the privileged-copy row, record evidence for each required aspect:

1. Host file staged into its private source child: yes / no / not run
2. Recursive host directory staged into its private source child: yes / no / not run
3. On-device private file copied into its distinct destination child: yes / no / not run
4. Recursive on-device private directory copied into its distinct destination child: yes / no / not run
5. Both staged sources and both copied destinations verified: yes / no / not run
6. Exact contract-owned copy roots cleaned and confirmed absent: yes / no / not run

## 5. Authority-failure evidence

- Root unavailable result:
- Root denied result:
- Root timeout/transport/unexpected result:
- Successful preflight followed by first privileged-operation denial:
- Privileged command failure result:
- Cleanup failure result:

Mark cases not physically exercised as `not run`; do not infer them from host
tests. Root-manager policy must not be changed automatically by the harness.

## 6. Cleanup and residual state

- Normal group children absent after cleanup: yes / no
- Controlled residual exact path:
- Residual matches one `cleanup-failure-<run-id>` child: yes / no
- Manual removal command captured externally: yes / no
- Exact residual absent after manual removal: yes / no
- Other residual package-private state observed: yes / no

Only contract-owned paths may appear here. Never record unrelated app-private
paths or directory contents.

## 7. Final qualification statement

- Physical rooted-device qualification: passed / failed / incomplete
- Unsupported production-supported operations:
- Genuine blockers:
- Required follow-up:

Do not mark Phase 6C.2 complete unless every production-supported privileged
operation has representative rooted-device evidence and cleanup is verified.
