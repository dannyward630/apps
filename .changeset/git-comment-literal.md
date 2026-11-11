---
"@servers/git-tracker-api": patch
---

Render untrusted pull request comment fields as bounded literal code while preserving stored identifiers and comment publication behavior.

<!-- public-changelog:start -->

Git Tracker now displays uploaded identifiers as literal text in pull request comments. Long identifiers are shortened in the comment; their full values remain available in execution details.

<!-- public-changelog:end -->
