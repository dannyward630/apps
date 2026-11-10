---
"@servers/mcpa-api": patch
---

Apply pending Managed CPA schema migrations in one transaction and record SHA-256 digests for every applied SQL file. Backfill legacy migration rows only after validating the pinned historical SQL baseline, and reject schema digest mismatches at startup and readiness checks.
