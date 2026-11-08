---
"@delinoio/anydash": patch
---

Enforce HTTPS-only downloads in the shell installer's wget fallback and explain unsupported wget versions.

<!-- public-changelog:start -->

The AnyDash shell installer now blocks insecure download redirects when it uses wget. If the installer reports that your wget does not support the required HTTPS options, install curl or GNU Wget.
<!-- public-changelog:end -->
