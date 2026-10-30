# ADR 0008: Explicit copy intents, previewed deletion, and execution parity

**Status:** Accepted; the manifest-driven implementation is present, with production qualification still bounded.

EmuChef distinguishes non-destructive Add or Update Copy, intentional Replace Destination Copy, and Mirror Copy, which removes destination-only files. Non-destructive transfer is the default; replacement or deletion is deliberate. Mirror Copy prepares a concrete deletion manifest before review, presents its scope, and revalidates it immediately before execution. An operation is blocked if its scope cannot be determined or revalidated safely.

The fake and Android adapters implement these policies from the same reviewed source and destination manifests. Host source entries are opened relative to a retained source-root descriptor without following symlinks, checked as regular files, and hashed against their reviewed SHA-256 while copied. Android stages each checked source entry before mutation; the selected-root helper opens staged files without following symlinks, verifies the reviewed digest, and checks the exact publication temporary file before rename. Android Mirror removes only the reviewed deletion set. Results distinguish success, partial mutation, and uncertain state; failure does not imply whole-setup rollback.

The host copy path requires Unix descriptor-relative no-follow filesystem operations. Hosts without those primitives reject source access rather than fall back to path-based opens; non-Unix host support is not qualified.

After final destination verification succeeds, a staging cleanup error remains pending cleanup and does not change deployment success. The execution report exposes this condition as an owner-visible warning while preserving the successful step and deployment result.

Automated tests and controlled Android virtual-device qualification establish only the exercised host, helper build, Android image, and device route. They do not establish support for every Android release, filesystem, root route, OEM configuration, physical device, or production application. Physical-device and broader production qualification remain separate. Interrupted operations are reported from observed state; the implementation does not provide whole-tree rollback or automatic resume. Pre-v1.0 authored and saved-setup schemas may change incompatibly, and obsolete test or qualification setups may be rejected and recreated without silently deleting user files.
