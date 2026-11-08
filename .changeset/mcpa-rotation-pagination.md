---
"@servers/mcpa-api": patch
---

Advance master-key rotation batches by row ID and make one bounded final pass for concurrent old-key reseals. Preserve the existing remaining-key report and incomplete-rotation exit status.

<!-- public-changelog:start -->

Large Managed CPA master-key rotations now avoid repeatedly scanning earlier batches. Rotation makes a final pass for credentials updated during the run and still reports any rows that need another run. Keep old keys configured until no rows remain under them.

<!-- public-changelog:end -->
