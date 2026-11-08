---
"@servers/git-tracker-api": patch
---

Skip audit expiry when the exact cutoff falls before PostgreSQL timestamptz range.

<!-- public-changelog:start -->

Git Tracker now accepts long audit retention periods without repeated maintenance errors.
<!-- public-changelog:end -->
