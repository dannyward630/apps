---
"@servers/mcpa-api": patch
---

Retain a failed final metering checkpoint after the model response and retry its original sequence and byte deltas in engine-owned background work until it commits or the engine stops. Make only one final attempt on the response path. Preserve the classified final status and avoid a second final sequence after a retry succeeds.

<!-- public-changelog:start -->

Model responses no longer wait for repeated metering retries when the usage store is temporarily unavailable. The service keeps retrying the final usage record while it runs.

<!-- public-changelog:end -->
