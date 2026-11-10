---
"@delinoio/anydash": patch
---

Cap numeric Retry-After waits at one day and use safe timestamp addition for provider, upload and API retry deadlines.

<!-- public-changelog:start -->

AnyDash respects provider rate limits without repeatedly polling when a service sends an unusually large retry delay.

<!-- public-changelog:end -->
