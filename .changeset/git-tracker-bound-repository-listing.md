---
"@servers/git-tracker-api": patch
---

Select only repository identity columns for listings, cap anonymous results at 100, and omit installation IDs from anonymous responses.

<!-- public-changelog:start -->

Public repository lists now return up to 100 repositories without GitHub App installation IDs. Authorized space lists still include all repositories in the space.

<!-- public-changelog:end -->
