---
"@delinoio/anydash": patch
---

Return not logged in before loading the encryption key when the collector has no device credentials.

<!-- public-changelog:start -->

Running `anydash logout` before login now reports that the device is not logged in without creating an encryption key or requiring one from the environment.

<!-- public-changelog:end -->
