---
"@servers/git-tracker-api": patch
---

Limit newly collected push ancestry to 1,000 first-parent commits and 20 seconds, using paged GitHub results with verified parent links.

<!-- public-changelog:start -->

Automatic push comparisons now check up to the 1,000 nearest first-parent commits. If your row has no completed measurement in that range, its comparison is unavailable. You can set an explicit baseline commit to compare with an older result.

<!-- public-changelog:end -->
