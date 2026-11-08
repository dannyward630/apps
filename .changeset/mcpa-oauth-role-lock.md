---
"@servers/mcpa-api": patch
---

Recheck resource management access under the resource lock when starting provider OAuth flows, including after a device provider call.

<!-- public-changelog:start -->

If your resource management access is removed while you connect a provider account, the connection attempt now stops without creating a pending flow.
<!-- public-changelog:end -->
