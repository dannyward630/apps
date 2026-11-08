---
"@servers/mcpa-api": patch
---

Park Delcore cardinality rejections after the first delivery attempt, process later tenants' events in the same worker cycle, and add a reason-filtered operator command to requeue the parked events after Delcore capacity is available.

<!-- public-changelog:start -->

Managed CPA now stops retrying usage events rejected for Delcore cardinality limits. Operators can restore those events after increasing Delcore capacity with `mcpa-api requeue-outbox --reason cardinality_exceeded`.

<!-- public-changelog:end -->
