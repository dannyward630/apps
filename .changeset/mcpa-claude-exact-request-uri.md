---
"@servers/mcpa-api": patch
---

Count the exact Claude provider request URI that the SDK writes when a configured base URL ends in a slash. Preserve HTTP/2 trace and request-body metering.

<!-- public-changelog:start -->

Claude calls through a provider base URL ending in `/` now count the complete provider request line in usage bytes.

<!-- public-changelog:end -->
