---
"@servers/anydash-api": patch
---

Reject collector logout with inactive or expired refresh credentials without discarding retained digests. Detect rotated refresh token reuse during logout and revoke the device credentials, matching refresh behavior and device rate limits.
