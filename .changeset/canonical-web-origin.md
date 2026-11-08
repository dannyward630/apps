---
"@servers/mcpa-api": patch
---

Canonicalize default console-origin ports and reject non-ASCII hosts before CORS and Origin checks.

<!-- public-changelog:start -->

The Managed CPA console accepts requests when its configured origin includes an explicit default port.
<!-- public-changelog:end -->
