# Tauri vs Flutter for EmuChef

Date: 2026-09-25

## Scope

This is a greenfield framework evaluation for both EmuChef proper and the Config Editor. It deliberately ignores the fact that the current applications are already implemented in Tauri.

The comparison preserves the current product/core requirements while treating "Tauri" references in the existing documents as implementation-specific rather than automatically authoritative. The framework-neutral requirements come from:

- `PRODUCT.md`
- `docs/architecture/runtime-ownership.md`
- `docs/architecture/editor-runtime.md`
- `docs/product/config-editor-authored-generation.md`
- `docs/product/phase-3c-accessibility-and-interaction-resilience.md`
- `docs/product/phase-3e-macos-packaging-and-release-readiness.md`

The user has explicitly stated that raising the minimum deployment target from macOS 11 to macOS 12 is acceptable, so the current Flutter macOS floor is not a deciding disadvantage.

## Framework-neutral product constraints

The relevant constraints are:

1. Rust remains the sole authoritative product runtime for authored-data validation, planning, execution, ADB/device authority, canonical serialization, and the existing JSONL sidecar protocol.
2. EmuChef proper is a guided, simulation-first desktop workflow for nontechnical users with strong accessibility, recovery, privacy, and fail-closed execution requirements.
3. The presentation layer should receive sanitized projections and opaque handles rather than raw serials, paths, plans, credentials, or execution authority.
4. Config Editor is a technical, form/editor-heavy authoring product with validation, undo/redo, canonical YAML, generators, device/profile authoring, and native file/device integration.
5. Both products need native macOS packaging, signing/notarization, bundled Rust runtime delivery, file dialogs, external-browser handoff, diagnostics/recovery storage, and child-process/runtime lifecycle management.
6. Windows and Linux are explicitly post-MVP, but the architecture should not make later desktop support needlessly expensive.
7. The two products stay separate and should not accidentally merge their frontend state or workflows.

## Tauri

### Core fit

Tauri is intentionally split between an HTML/WebView frontend and Rust host code. Its command system provides an explicit bridge from the frontend into Rust.

Primary source: https://v2.tauri.app/develop/calling-rust/

Tauri 2 also has a runtime capability/permission system. A window or webview can be given no IPC access or only selected command/plugin permissions, and runtime authority checks requests before commands run.

Primary sources:
- https://v2.tauri.app/reference/acl/capability/
- https://v2.tauri.app/security/runtime-authority/

That directly matches EmuChef's desired separation between presentation/user intent and native/runtime authority. The separation is not automatic proof that command implementations are safe, but the framework gives the application an enforceable front-door boundary.

Tauri also supports strict CSP configuration for the web frontend, reducing the effect of common web-content injection failures when configured correctly.

Primary source: https://v2.tauri.app/security/csp/

### Rust runtime and sidecar fit

Tauri has first-class support for bundling architecture-specific external binaries and launching them as sidecars. External binaries use target-triple naming and shell permissions can constrain which programs and argument forms the frontend can execute.

Primary sources:
- https://v2.tauri.app/develop/sidecar/
- https://v2.tauri.app/reference/javascript/shell/

This is unusually aligned with the current EmuChef core, whose authoritative runtime already exists as a Rust executable with a long-running JSONL sidecar contract. A Tauri host can either keep the sidecar boundary or call shared Rust crates directly for host-only functionality without requiring a C ABI or language-binding layer.

### UI and accessibility

Tauri uses the operating system WebView; on macOS this is WKWebView/WebKit.

Primary sources:
- https://v2.tauri.app/concept/architecture/
- https://v2.tauri.app/reference/webview-versions/

For EmuChef's UI, HTML semantics, native browser focus behavior, form controls, ARIA, CSS media features, text reflow, and zoom are a strong match to the explicit accessibility requirements. This still requires careful app implementation and manual VoiceOver qualification; Tauri does not make accessibility correct by itself.

The tradeoff is renderer/version variation across platforms because Tauri uses each OS's webview instead of shipping one renderer.

### Testing

Tauri supports unit/integration tests through a mock runtime, frontend IPC mocking, and desktop end-to-end testing through WebDriver-related tooling. The frontend can also use the normal web testing ecosystem.

Primary sources:
- https://v2.tauri.app/develop/tests/
- https://v2.tauri.app/develop/tests/mocking/

The main testing weakness is that a web-mocked frontend or Tauri mock runtime does not prove the real native/WebView integration; packaged-GUI qualification remains necessary. That is already consistent with EmuChef's evidence model.

### Packaging

Tauri provides native app/DMG bundling, macOS minimum-version configuration, code signing, Developer ID, and notarization integration.

Primary sources:
- https://v2.tauri.app/distribute/
- https://v2.tauri.app/distribute/macos-application-bundle/
- https://v2.tauri.app/distribute/sign/macos/

Tauri uses the system web renderer rather than embedding a rendering engine, reducing framework payload size compared with a Flutter desktop bundle.

Primary source: https://v2.tauri.app/concept/architecture/

## Flutter

### Core fit

Flutter desktop apps are Dart applications rendered by a Flutter engine. Flutter can call native code through platform plugins/channels, and its current recommended native-code packaging path supports FFI packages and build hooks.

Primary sources:
- https://docs.flutter.dev/resources/faq
- https://docs.flutter.dev/packages-and-plugins/developing-packages
- https://docs.flutter.dev/platform-integration/bind-native-code

There are three plausible Rust integration models for EmuChef:

1. Dart launches the existing `emuchef --sidecar` process and speaks JSONL directly.
2. A macOS Swift/native host or Flutter plugin owns the sidecar and exposes a narrow channel to Dart.
3. Rust is compiled as a native library behind a C ABI/FFI layer and called in-process from Dart.

For current EmuChef semantics, the sidecar models preserve process lifecycle/restart isolation best. An FFI conversion would create a new ABI/binding surface and would change the failure/lifecycle model of the current Rust runtime.

Flutter/Dart can directly access files, directories, sockets, HTTP, and child processes via `dart:io`.

Primary sources:
- https://api.dart.dev/dart-io/
- https://api.dart.dev/dart-io/Process-class.html

This is powerful, but it means Flutter does not naturally enforce EmuChef's "presentation has no filesystem/process/network authority" boundary. A Dart UI can be architecturally disciplined, or all privileged operations can be hidden behind a native plugin/channel, but the framework does not provide a Tauri-like per-window command capability system that prevents ordinary Dart code from using `dart:io`.

That does not make Flutter insecure. Flutter avoids the ordinary HTML/JavaScript/XSS execution model entirely. The narrower point is that EmuChef's desired *authority decomposition* maps less directly onto Flutter's default desktop application model.

### UI and accessibility

Flutter controls its own rendering, which gives highly consistent visuals across desktop platforms and strong control over animation, layout, and custom interaction.

Flutter has first-class accessibility/semantics APIs, VoiceOver support, and automated accessibility guideline tests for labels, target sizes, and contrast.

Primary sources:
- https://docs.flutter.dev/ui/accessibility
- https://docs.flutter.dev/ui/accessibility/accessibility-testing

This is a viable accessibility foundation. However, EmuChef proper's requirements are especially DOM-like: semantic landmarks/collections, field relationships, native validation relationships, focus-contained dialogs, forced-colors behavior, zoom/reflow, and screen-reader navigation. Those can be implemented in Flutter, but they require deliberately modeling Flutter's semantics/focus tree rather than relying on native HTML semantics.

Flutter's macOS native Platform Views remain documented as not fully functional in the current release, including missing gesture support.

Primary source: https://docs.flutter.dev/platform-integration/macos/platform-views/

That is not a blocker for current EmuChef because neither product fundamentally requires embedded native views, but it reduces the attractiveness of "drop down to AppKit views" as a general escape hatch.

### Testing

Flutter has excellent framework-native unit, widget, golden, integration, and accessibility-guideline testing.

Primary sources:
- https://docs.flutter.dev/testing/overview
- https://docs.flutter.dev/cookbook/testing/widget/introduction
- https://docs.flutter.dev/ui/accessibility/accessibility-testing

Its official `integration_test` package cannot interact with native platform UI such as permission dialogs or platform views, so native integration still requires another test strategy/manual packaged qualification.

Primary source: https://docs.flutter.dev/testing/overview

This is a real strength for deterministic visual/component testing, especially if EmuChef later develops a more custom visual design.

### Packaging

Flutter currently supports macOS 12 and later, so a macOS 12 deployment target removes the prior compatibility mismatch.

Primary source: https://docs.flutter.dev/reference/supported-platforms

Flutter's macOS packaging goes through the generated Xcode Runner project. Flutter documents App Sandbox, entitlements, Hardened Runtime, signing, and notarization workflows.

Primary sources:
- https://docs.flutter.dev/platform-integration/macos/building
- https://docs.flutter.dev/deployment/macos

Bundling native libraries is supported, including current FFI build hooks. A standalone Rust sidecar executable can also be placed in the macOS bundle with Xcode/build scripting, but Flutter does not document a sidecar lifecycle/bundling abstraction comparable to Tauri's `externalBin` plus scoped shell permissions.

Primary sources:
- https://docs.flutter.dev/platform-integration/bind-native-code
- https://docs.flutter.dev/platform-integration/macos/building

## Application-specific evaluation

### EmuChef proper

Tauri is the stronger fit.

The most important reason is not existing implementation. It is that EmuChef proper intentionally treats the UI as an untrusted/sanitized presentation surface while retaining sensitive authority in Rust/native code. Tauri's WebView-to-Rust IPC and capability model directly encode that shape. Flutter can reproduce it, but only by introducing a custom native authority layer or accepting that the Dart process itself has general filesystem/process/network capabilities.

The second reason is accessibility. EmuChef proper is primarily a guided forms/status/review application, not a graphics-heavy custom-rendered application. HTML/WebView semantics are closer to the product's stated keyboard, VoiceOver, landmarks, validation, zoom/reflow, reduced-motion, and high-contrast requirements than a custom Flutter semantics tree.

The third reason is runtime/process packaging. The existing Rust CLI/sidecar is not accidental UI glue; it is the product core. Tauri has first-class external-binary support and Rust host integration that preserve it with minimal architectural translation.

Flutter's main advantages here are more deterministic cross-platform rendering and stronger built-in widget/golden tests. Those are real but secondary to EmuChef proper's trust-boundary and accessibility requirements.

### Config Editor

The result is closer, but Tauri is still the stronger fit.

Flutter becomes more attractive for the editor because:
- the users are technical;
- the UI may become denser and more custom;
- widget/golden testing is strong;
- renderer consistency would help later Windows/Linux support.

However, the current and planned Config Editor is still fundamentally a desktop authoring/forms/text application: canonical YAML previews, dedicated app-definition/profile editors, aliases, device-plan assistance, collision review, native file selection, network-source analysis, and Rust-authoritative save/validation. These workloads fit HTML/React well, and the same Rust/sidecar authority boundary remains useful.

Flutter would become the stronger candidate if the Config Editor evolves into a graph/canvas-heavy visual tool, relies on highly custom interactive visualization, or cross-platform pixel-identical rendering becomes a dominant product requirement. Those are not current requirements.

## Greenfield conclusion

With macOS 12 accepted:

- **EmuChef proper:** Tauri has a clear architectural advantage.
- **Config Editor:** Tauri has a moderate architectural advantage; Flutter is credible but does not currently offer a product-requirement advantage large enough to offset the extra native/Rust boundary design it would require.
- **Shared Rust core:** Tauri is the more direct host because Rust is a first-class half of the framework rather than a foreign-language integration.
- **If one UI framework is desired for both applications:** Tauri is the better common denominator for the current product definition.

The deciding criteria are the trust/authority model, sidecar/Rust integration, and accessibility shape—not sunk implementation cost or the previous macOS 11 deployment target.
