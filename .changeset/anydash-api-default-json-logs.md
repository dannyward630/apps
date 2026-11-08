---
"@servers/anydash-api": patch
---

Default AnyDash API tracing logs to JSON when LOG_FORMAT is unset or empty, while preserving explicit text and json overrides and the text default for other Rust services.
