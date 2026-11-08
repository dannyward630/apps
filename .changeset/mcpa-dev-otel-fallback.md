---
"@servers/mcpa-api": patch
---

Apply development OTEL settings to the trace exporter and resource SDK while preserving process overrides.

<!-- public-changelog:start -->

Managed CPA API now exports development traces to the OTLP endpoint set in its `.env` file when `--dev-env` is used.
<!-- public-changelog:end -->
