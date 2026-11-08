---
"@servers/mcpa-api": patch
---

Close the data-plane engine after the HTTP drain so active model calls retain credentials and token persistence.

<!-- public-changelog:start -->

Model calls already in progress can finish during a server restart when they complete within the graceful shutdown period.
<!-- public-changelog:end -->
