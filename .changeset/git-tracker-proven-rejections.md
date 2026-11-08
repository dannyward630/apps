---
"@servers/git-tracker-api": patch
---

Classify confirmed pull request and branch commit rejections as failed preconditions, separate from GitHub provider failures.

<!-- public-changelog:start -->

Git Tracker now reports a failed precondition when a commit is confirmed to be outside the selected pull request or branch, or when the pull request targets a different branch. Temporary GitHub failures still return an unavailable error.
<!-- public-changelog:end -->
