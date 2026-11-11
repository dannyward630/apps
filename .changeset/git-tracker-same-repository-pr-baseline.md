---
"@servers/git-tracker-api": patch
---

Allow explicit baseline commits for same-repository pull request uploads authenticated with Actions OIDC. Keep fork credential baseline restrictions and other authorization bindings.

<!-- public-changelog:start -->

Git Tracker now accepts the Action's `baseline` input for pull request jobs that run with an OIDC credential from the connected repository. Fork pull request credentials still cannot set a baseline override.

<!-- public-changelog:end -->
