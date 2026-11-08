---
"@servers/mcpa-api": patch
---

Restrict call authorization ACL reads to the key issuer's direct and group entries, and cap new ACL principals at 1,000 per resource.

<!-- public-changelog:start -->

Model calls remain responsive when a resource has a large access list. A resource now accepts up to 1,000 access list entries; managers can still update existing entries at the limit.
<!-- public-changelog:end -->
