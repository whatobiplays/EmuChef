# Phase 6F Physical-Device Qualification Matrix

Generated from `docs/testing/device-qualification/` definitions and immutable physical evidence.

## device-target-sha256:8b9f6cf2dc8831a9188c2ad1d85b8a83ea8e8baf0d674943b7d5e9925f047c62

- Configuration: AYANEO Synthetic Pocket S2, Android 15 (API 35), arm64-snapdragon, non_root, usb3
- Authored profile: ayaneo.pocket_s2
- Support tier: limited

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| retroarch-plus-bios | qualified | qualification-run-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa (2026-08-20T00:00:00Z) | — |
| obtainium-install | stale | qualification-run-sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc (2026-08-19T00:00:00Z) | no current compatible evidence; historical valid evidence exists |
| xaniteog-install | qualified | qualification-run-sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee (2026-08-18T00:00:00Z) | — |
| rom-library-sync | failed | qualification-run-sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb (2026-08-21T00:00:00Z) | execution-report failed |

## device-target-sha256:2565a33f08b4d19600d5a7cf3039fa1c250bb93384e2087fae3fd2d56512faec

- Configuration: AYANEO Synthetic Pocket Air Mini, Android 14 (API 34), arm64-mtk, non_root, usb2
- Authored profile: ayaneo.pocket_air_mini
- Support tier: unqualified

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| xaniteog-install | deferred | — | explicitly_deferred |
| rom-library-sync | missing | — | no applicable valid physical evidence |
