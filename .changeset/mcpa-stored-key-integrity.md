---
"@servers/mcpa-api": patch
---

Refuse service startup when encrypted rows reference a master key ID that is absent from the configured keyring, before constructing request handlers or workers.
