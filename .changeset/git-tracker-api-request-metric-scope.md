---
"@servers/git-tracker-api": patch
---

Limit the Git Tracker HTTP request counter used by the API error-rate alert to service API paths, count CORS-rejected API requests, and record bounded webhook response outcomes separately.

<!-- public-changelog:start -->

Git Tracker monitoring now measures the API error rate without health checks, metric scrapes, or webhook traffic. Webhook success, rejection, and failure counts are available separately from startup.

<!-- public-changelog:end -->
