---
"@delinoio/anydash": patch
---

Avoid reset queries for newly discovered session files, add indexed path deletes through a forward SQLite migration, and count skipped records only in files visited under enabled source paths.

<!-- public-changelog:start -->

AnyDash clears partial aggregation warnings after you remove a malformed session file or disable its source path. Initial scans of Claude Code session files also avoid unnecessary work when local history is large.

<!-- public-changelog:end -->
