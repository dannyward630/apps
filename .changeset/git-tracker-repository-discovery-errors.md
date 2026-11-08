---
"@servers/git-tracker-api": patch
---

Preserve GitHub discovery failure classes when connecting a repository, while retaining locked metadata refresh denial and redacting provider content.

<!-- public-changelog:start -->

Git Tracker now reports invalid repository names, missing repositories, and GitHub outages separately when connecting a repository.
<!-- public-changelog:end -->
