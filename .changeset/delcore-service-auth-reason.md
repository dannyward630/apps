---
"@servers/delcore-api": patch
---

Add a distinct Delcore error reason for service authentication failures during API key authorization.

<!-- public-changelog:start -->

The Delcore API now returns a distinct error reason when a service bearer token cannot be authenticated during API key authorization. Invalid customer key responses keep the same generic error.
<!-- public-changelog:end -->
