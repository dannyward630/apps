---
"@servers/git-tracker-api": patch
"@apps/delino-docs": patch
---

Return not found for deletions that match no execution or dataset, audit only actual removals, and limit dataset retry markers to executions that held the deleted dataset. Update the public deletion guide and translations.

<!-- public-changelog:start -->

Git Tracker now reports when an execution or dataset selected for deletion has no measurements to remove. Deleting a dataset no longer prevents other executions that never held it from uploading it, and their comparisons treat it as new data.

<!-- public-changelog:end -->
