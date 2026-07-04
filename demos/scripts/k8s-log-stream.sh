#!/usr/bin/env bash
set -euo pipefail

delay="${JGREP_DEMO_DELAY:-0.45}"

emit() {
  printf '%s\n' "$1"
  sleep "$delay"
}

emit '{"@timestamp":"2026-07-04T10:15:00Z","log":{"level":"INFO"},"kubernetes":{"namespace":"shop","pod":"checkout-7fbf9d7c8f-x9q2m"},"service":{"name":"checkout"},"message":"request accepted","trace":{"id":"trc-1001"}}'
emit '{"@timestamp":"2026-07-04T10:15:01Z","log":{"level":"WARN"},"kubernetes":{"namespace":"shop","pod":"checkout-7fbf9d7c8f-x9q2m"},"service":{"name":"checkout"},"message":"retrying payment provider","trace":{"id":"trc-1002"}}'
emit '{"@timestamp":"2026-07-04T10:15:02Z","log":{"level":"ERROR"},"kubernetes":{"namespace":"shop","pod":"checkout-7fbf9d7c8f-x9q2m"},"service":{"name":"checkout"},"message":"payment declined","trace":{"id":"trc-1003"}}'
emit '{"@timestamp":"2026-07-04T10:15:03Z","log":{"level":"INFO"},"kubernetes":{"namespace":"shop","pod":"cart-65bfcbf88d-pm7xz"},"service":{"name":"cart"},"message":"cart updated","trace":{"id":"trc-1004"}}'
emit '{"@timestamp":"2026-07-04T10:15:04Z","log":{"level":"ERROR"},"kubernetes":{"namespace":"shop","pod":"checkout-7fbf9d7c8f-x9q2m"},"service":{"name":"checkout"},"message":"fallback charge failed","trace":{"id":"trc-1005"}}'
