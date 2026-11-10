---
"@servers/anydash-api": patch
"@servers/delcore-api": patch
"@servers/delitools-api": patch
"@servers/devbase-api": patch
"@servers/git-tracker-api": patch
"@servers/perfine-api": patch
---

Keep the shared Rust server transport running after transient listener accept errors, with an interruptible retry delay for resource failures. Preserve existing connection and graceful shutdown handling.

<!-- public-changelog:start -->

Rust APIs now recover from temporary connection acceptance failures and resume serving when resources become available.

<!-- public-changelog:end -->
