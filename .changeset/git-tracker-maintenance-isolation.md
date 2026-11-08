---
"@servers/git-tracker-api": patch
---

Isolate repository maintenance and finalized-space deletion failures, count each failed operation once, and keep audit expiration running after earlier failures.

<!-- public-changelog:start -->

Git Tracker now continues updating other repositories and expiring old audit records when maintenance fails for one repository.

<!-- public-changelog:end -->
