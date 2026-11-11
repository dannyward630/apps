---
"@servers/git-tracker-api": patch
---

Validate deterministic upload input and execution conflicts before GitHub ancestry lookup. Keep authorization, locking, valid-upload resolution and completion behavior unchanged.

<!-- public-changelog:start -->

Git Tracker now reports invalid or conflicting upload data as a client error even when GitHub is unavailable. Correct the reported field or conflict and retry the upload.

<!-- public-changelog:end -->
