---
"@servers/mcpa-api": patch
---

Count provider request lines after the transport reports written headers. Failed dials, TLS handshakes, and requests cancelled before headers no longer add a request-line delta to usage checkpoints.

<!-- public-changelog:start -->

Provider connection failures no longer charge for an HTTP request line that was never sent.

<!-- public-changelog:end -->
