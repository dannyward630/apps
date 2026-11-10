---
"@servers/git-tracker-api": patch
---

Create one server-generated correlation ID at the Git Tracker HTTP boundary and reuse it across responses, request logs and existing audit records. Log webhook rejections once with bounded categories and no request secrets.

<!-- public-changelog:start -->

Git Tracker API responses now include a correlation ID, including health checks, webhook failures and request rejections. You can use that ID when reporting an API error.

<!-- public-changelog:end -->
