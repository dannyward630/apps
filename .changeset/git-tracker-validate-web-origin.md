---
"@servers/git-tracker-api": patch
---

Reject malformed Git Tracker WEB_ORIGIN values at startup before connecting to PostgreSQL.

<!-- public-changelog:start -->

Git Tracker now reports a configuration error at startup when its web origin includes a path or trailing slash.
<!-- public-changelog:end -->
