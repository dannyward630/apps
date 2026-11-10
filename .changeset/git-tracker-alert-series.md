---
"@servers/git-tracker-api": patch
---

Initialize Git Tracker's three alerted event counter series at zero during service startup, so Prometheus can observe their first increments after an initial scrape.

<!-- public-changelog:start -->

Git Tracker now exposes its failure and interruption metrics from startup. Monitoring can detect their first recorded event after an initial scrape.

<!-- public-changelog:end -->
