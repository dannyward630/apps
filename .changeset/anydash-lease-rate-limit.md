---
"@delinoio/anydash": patch
---

Keep a future provider rate-limit deadline when a collector device re-acquires its connection lease. First polls and ordinary failure retries remain prompt after acquisition.

<!-- public-changelog:start -->

AnyDash waits for a provider's requested retry time even if the collector restarts or regains the connection lease.

<!-- public-changelog:end -->
