---
"@servers/mcpa-api": patch
---

Reset the credential listener retry delay after PostgreSQL LISTEN succeeds.

<!-- public-changelog:start -->

Credential updates from another server resume sooner after an isolated database connection drop.
<!-- public-changelog:end -->
