# Device Qualification Evidence

This directory is reserved for immutable, validated physical-device qualification bundles.
Each recorded run lives in its own `qualification-run-sha256:<digest>/` directory with `evidence.json` and, when present, a digest-bound `execution-report.json`.
Synthetic fixtures belong only under `tests/fixtures/device-qualification/` and must never be copied here.
The production-bound harness intentionally contains no physical evidence bundles yet.
