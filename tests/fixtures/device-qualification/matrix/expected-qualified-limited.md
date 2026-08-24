# Device Qualification Matrix

Generated from `docs/testing/device-qualification/` definitions and immutable physical evidence.

## device-target-sha256:8b9f6cf2dc8831a9188c2ad1d85b8a83ea8e8baf0d674943b7d5e9925f047c62

- Configuration: AYANEO Synthetic Pocket S2, Android 15 (API 35), arm64-snapdragon, non_root, usb3
- Authored profile: ayaneo.pocket_s2
- Support tier: limited

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| retroarch-plus-bios | qualified | qualification-run-sha256:41c5416c2d52c3f246c6e2f68ddd987a088f3b70f656689101b364322fd99fb4 (2026-08-20T00:00:00Z) | — |
| obtainium-install | stale | qualification-run-sha256:3c0c3f963c4b3610eca24da81a7984b214884c9654ccbdce4fb397481f580410 (2026-08-19T00:00:00Z) | no current compatible evidence; historical valid evidence exists |
| xaniteog-install | qualified | qualification-run-sha256:c7c297fa674427c9e32c91f5aeb319c03d3951734235d38a4b83f4c42f15f1aa (2026-08-18T00:00:00Z) | — |
| rom-library-sync | failed | qualification-run-sha256:9390a13d43659237d02f4edd9ab549e5c18186e5fcf9c2d450d39a04a3bc2f30 (2026-08-21T00:00:00Z) | execution-report failed |

## device-target-sha256:2565a33f08b4d19600d5a7cf3039fa1c250bb93384e2087fae3fd2d56512faec

- Configuration: AYANEO Synthetic Pocket Air Mini, Android 14 (API 34), arm64-mtk, non_root, usb2
- Authored profile: ayaneo.pocket_air_mini
- Support tier: unqualified

| Workflow | State | Current evidence | Reason / limitation |
|---|---|---|---|
| xaniteog-install | deferred | — | explicitly_deferred |
| rom-library-sync | missing | — | no applicable valid physical evidence |
