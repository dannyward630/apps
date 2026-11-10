---
"@servers/mcpa-api": patch
---

Classify caller-aborted model calls and their failure metrics without attributing cancellation to Delcore or the provider.

<!-- public-changelog:start -->

Model calls cancelled by the client now appear as client cancellations in call records and monitoring, rather than service or provider failures.

<!-- public-changelog:end -->
