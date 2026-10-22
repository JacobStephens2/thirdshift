# Official OTLP examples

`logs.json`, `events.json` and `trace.json` are unmodified examples from
[opentelemetry-proto at 09769935deb8b2ebcf73ad1e2142b691d17bf70b](https://github.com/open-telemetry/opentelemetry-proto/tree/09769935deb8b2ebcf73ad1e2142b691d17bf70b/examples),
retrieved on 2026-10-21. Copyright OpenTelemetry Authors; Apache-2.0
([LICENSE](LICENSE)). Tests compare their decoded values through the module and
OpenTelemetry's official protocol types. Uppercase IDs normalize to lowercase.

`file-logs.jsonl` and `file-spans.jsonl` are the four log and four trace lines
from the [File Exporter specification at e30cae5ce80e7bf9a8f051f20018b764d4bac029](https://github.com/open-telemetry/opentelemetry-specification/blob/e30cae5ce80e7bf9a8f051f20018b764d4bac029/specification/protocol/file-exporter.md),
also Apache-2.0. Metrics are outside this module's scope.
