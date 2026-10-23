# ARMSX1 recipe qualification

The authored ARMSX1 App Definition owns package identity and the GitHub release asset policy. The `app.armsx1.install` Recipe resolves that App Artifact and installs it with replacement enabled, then verifies package presence. It does not skip an install because a prior version is present.

Automated evidence is split by capability. Mandatory offline qualification exercises the checked-in App and Recipe through production planning, review, fixture release resolution, APK manifest validation, and sandbox execution. It proves stable matching release selection, failure handling, and repeated replacement behavior without network or device access.

Live emulator qualification is a distinct ignored test and is never part of normal `make test` or CI. It is available only through the explicitly named `make qualify-armsx1-emulator` target with exact AVD identity, dedicated/disposable operator attestation, package allowlist, API 36, ARM64, boot-complete, online, and non-root checks. It uses production HTTPS release resolution and the real ADB executor. A live result remains unverified until this guarded test completes against the operator-identified emulator.

Emulator results do not qualify physical handhelds, packaged GUI workflows, production release readiness, or deferred Phase 6D/6E acceptance.
