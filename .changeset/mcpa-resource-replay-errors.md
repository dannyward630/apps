---
"@servers/mcpa-api": patch
---

Align resource replay and idempotency conflict responses with the Managed CPA API contract.

<!-- public-changelog:start -->

Replaying a resource creation request after deleting that resource now returns not found. Reusing a request key with different details now returns the Connect `failed_precondition` code for resource creation and call-key issuance.
<!-- public-changelog:end -->
