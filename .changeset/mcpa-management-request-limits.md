---
"@servers/mcpa-api": patch
---

Bound management request messages and wire bodies before authentication.

<!-- public-changelog:start -->

Managed CPA now rejects oversized management requests before authentication. Keep each message within 256 KiB and the HTTP request body within 1 MiB.
<!-- public-changelog:end -->
