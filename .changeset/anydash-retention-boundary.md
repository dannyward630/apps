---
"@delinoio/anydash": patch
---

Use the exact retention cutoff when refreshing local usage buckets so partial boundary buckets stay pruned.

<!-- public-changelog:start -->

AnyDash stops retrying usage uploads for buckets outside the 90-day retention window. After a successful sync, pending usage can clear normally.
<!-- public-changelog:end -->
