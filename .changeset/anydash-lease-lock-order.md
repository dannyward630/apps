---
"@servers/anydash-api": patch
---

Lock shared connections in ID order when ending device leases or clearing expired leases, avoiding deadlocks with concurrent heartbeats while keeping failover accounting unchanged.

<!-- public-changelog:start -->

Deleting a device now completes reliably when another connected device checks in at the same time.

<!-- public-changelog:end -->
