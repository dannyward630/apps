---
"@servers/git-tracker-api": patch
"@repo/protos": patch
---

Validate Git Tracker space IDs as canonical lowercase UUID v7 values before Delcore authorization. Preserve Delcore invalid-argument responses through the shared authority client and document the space ID fields in the protocol.

<!-- public-changelog:start -->

Git Tracker now reports invalid space IDs as request errors instead of temporary service failures. Send lowercase UUID v7 space IDs when connecting or listing repositories.

<!-- public-changelog:end -->
