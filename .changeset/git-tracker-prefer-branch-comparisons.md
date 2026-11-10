---
"@servers/git-tracker-api": patch
"@apps/delino-docs": patch
---

Prefer completed branch executions for both commits in explicit comparisons, with completed pull request executions as the fallback. Update the public comparison guide and its translations.

<!-- public-changelog:start -->

When you compare two commits in Git Tracker, a completed branch measurement now takes precedence over a pull request measurement for the same commit. If there is no completed branch measurement, the comparison still uses the latest completed pull request measurement.

<!-- public-changelog:end -->
