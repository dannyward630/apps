---
"@servers/git-tracker-api": patch
---

Skip optional PR publication for push executions and classify publication lock and pool timeouts as GitHub failures.

<!-- public-changelog:start -->

Git Tracker no longer reports a PR comment warning for push measurements. A delayed PR comment also no longer reports a measurement storage failure.
<!-- public-changelog:end -->
