---
"@servers/git-tracker-api": patch
---

Accept signed GitHub webhook bodies up to 25 MiB, parse only consumed fields after raw-body verification, and process large installation repository removals in one scoped database update. Keep the Connect RPC body limit at 8 MiB.

<!-- public-changelog:start -->

Git Tracker now accepts larger GitHub App events. Large repository removal events can disconnect the affected repositories and make their retained data private.

<!-- public-changelog:end -->
