---
"@delinoio/anydash": patch
---

Preserve the earliest Claude Code session timestamp across retention pruning and later transcript appends, while counting only sessions that began inside the 90-day window.

<!-- public-changelog:start -->

AnyDash no longer counts an older Claude Code session again when its transcript receives new entries inside the 90-day usage window.
<!-- public-changelog:end -->
