# Device Qualification Evidence

This directory is reserved for immutable, validated physical-device qualification bundles.
Each recorded run lives in its own `qualification-run-sha256:<digest>/` directory with `evidence.json` and, when captured, a digest-bound `execution-report.json` that passes the global version-2 report schema. A valid run requires a non-simulated `real_device` report with a reviewed plan ID and SHA-256 digest. An invalid `not_observed` run may retain a simulated report as audit evidence or omit the report when capture was unavailable.
Synthetic fixtures belong only under `tests/fixtures/device-qualification/` and must never be copied here.
The production-bound harness intentionally contains no physical evidence bundles yet.

Version-1 reports and malformed version-2 reports cannot support new qualification records. Recovery may preserve older report bytes in invalid audit records, but their status is not treated as proof. Existing bundles with checksum- and identity-valid version-1 reports remain byte-for-byte unchanged and are excluded from the current evidence projection. Direct recording rejects version-1 reports, and duplicate run IDs across historical and current records invalidate the inventory.
