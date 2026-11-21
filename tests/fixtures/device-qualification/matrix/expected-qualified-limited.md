# Device Qualification Matrix

Generated from `docs/testing/device-qualification/` definitions and immutable physical evidence.

## device-target-sha256:8b9f6cf2dc8831a9188c2ad1d85b8a83ea8e8baf0d674943b7d5e9925f047c62

- Configuration: AYANEO Synthetic Pocket S2, Android 15 (API 35), arm64-snapdragon, non_root, usb3
- Authored profile: ayaneo.pocket_s2
- Support tier: limited

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| retroarch-plus-bios | qualified | qualification-run-sha256:e837facf8f5318c3c15807d4ba4b85f81688e1aa1ac523c69cab31df305d6bb8 (2026-08-20T00:00:00Z) | — |
| obtainium-install | stale | qualification-run-sha256:8ac2a6e276fbf70fb873c352be42a0760cb9ff24c755006972253cac05a37e06 (2026-08-19T00:00:00Z) | no current compatible evidence; historical valid evidence exists |
| xaniteog-install | qualified | qualification-run-sha256:a494fc37cc1d625d402b82e489554dfe09a3a2125dbcbb9f261b783eb73dad42 (2026-08-18T00:00:00Z) | — |
| rom-library-sync | failed | qualification-run-sha256:ce4f9a3f51ea393da0148a92e4b560fbbd1fd6959ce356b20383e505c69b66a4 (2026-08-21T00:00:00Z) | execution-report failed |

## device-target-sha256:2565a33f08b4d19600d5a7cf3039fa1c250bb93384e2087fae3fd2d56512faec

- Configuration: AYANEO Synthetic Pocket Air Mini, Android 14 (API 34), arm64-mtk, non_root, usb2
- Authored profile: ayaneo.pocket_air_mini
- Support tier: unqualified

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| xaniteog-install | deferred | — | explicitly_deferred |
| rom-library-sync | missing | — | no applicable valid physical evidence |
