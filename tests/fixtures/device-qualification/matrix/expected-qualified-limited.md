# Phase 6F Physical-Device Qualification Matrix

Generated from `docs/testing/device-qualification/` definitions and immutable physical evidence.

## synthetic-pocket-s2

- Configuration: AYANEO Synthetic Pocket S2, Android 15 (API 35), arm64-snapdragon, non_root, usb3
- Authored profile: ayaneo.pocket_s2
- Support tier: limited

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| retroarch-plus-bios | qualified | phase-6f-run-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa (2026-08-20T00:00:00Z) | — |
| obtainium-install | stale | phase-6f-run-sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc (2026-08-19T00:00:00Z) | no current compatible evidence; historical valid evidence exists |
| xaniteog-install | qualified | phase-6f-run-sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee (2026-08-18T00:00:00Z) | — |
| rom-library-sync | failed | phase-6f-run-sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb (2026-08-21T00:00:00Z) | execution-report failed |

## synthetic-air-mini

- Configuration: AYANEO Synthetic Pocket Air Mini, Android 14 (API 34), arm64-mtk, non_root, usb2
- Authored profile: ayaneo.pocket_air_mini
- Support tier: unqualified

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| xaniteog-install | deferred | — | explicitly_deferred |
| rom-library-sync | missing | — | no applicable valid physical evidence |
