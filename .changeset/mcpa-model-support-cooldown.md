---
"@servers/mcpa-api": patch
---

Keep registered credentials available after a provider rejects a model. The provider's response remains visible to the caller, and later requests can try the model again without waiting for a cooldown.

<!-- public-changelog:start -->

If a provider rejects a model, you can retry the model with the same registered credential. Managed CPA continues to return the provider's original error response.

<!-- public-changelog:end -->
