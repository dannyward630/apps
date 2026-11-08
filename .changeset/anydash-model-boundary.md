---
"@delinoio/anydash": patch
---

Normalize local usage model keys at parsing and repair invalid keys saved by older collectors before upload.

<!-- public-changelog:start -->

AnyDash now uploads local usage reported with empty or invalid model names under the `unknown` model. Older pending usage with those names can upload after the collector updates.
<!-- public-changelog:end -->
