---
"@servers/mcpa-api": patch
---

Keep GetUsage pagination stable when a daily aggregate row disappears after a checkpoint or retention. Encode the row position in new opaque cursors, validate cursor input, and accept existing UUID cursors for one release while their rows remain available.

<!-- public-changelog:start -->

Usage pages now continue after a row is removed during browsing. Existing pagination links remain usable while their referenced row exists.

<!-- public-changelog:end -->
