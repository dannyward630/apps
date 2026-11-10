---
"@servers/mcpa-api": patch
---

Validate usage and call-record model, provider and status filters before querying PostgreSQL. Reject empty, oversized, invalid UTF-8 or control-containing model values, more than 100 model values, and undefined enum values. Preserve valid enum zero values and duplicate filters.

<!-- public-changelog:start -->

Usage and call-record requests now report invalid filters as input errors when a model name or provider or status value is invalid.

<!-- public-changelog:end -->
