#!/usr/bin/env bash
set -euo pipefail

delay="${JGREP_DEMO_DELAY:-0.45}"
loops="${JGREP_DEMO_LOOPS:-4}"

emit() {
  printf '%s\n' "$1"
  sleep "$delay"
}

for i in $(seq 1 "$loops"); do
  minute=$(printf '%02d' $((14 + i)))
  checkout_pod="checkout-7fbf9d7c8f-x9q2m"
  cart_pod="cart-65bfcbf88d-pm7xz"
  worker_pod="payment-worker-6b7d9df56d-mh${i}qk"

  emit "{\"@timestamp\":\"2026-07-04T10:${minute}:00Z\",\"log\":{\"level\":\"INFO\"},\"kubernetes\":{\"namespace\":\"shop\",\"pod\":\"${checkout_pod}\"},\"service\":{\"name\":\"checkout\"},\"message\":\"request accepted\",\"trace\":{\"id\":\"trc-${i}001\"}}"
  emit "{\"@timestamp\":\"2026-07-04T10:${minute}:01Z\",\"log\":{\"level\":\"DEBUG\"},\"kubernetes\":{\"namespace\":\"shop\",\"pod\":\"${worker_pod}\"},\"service\":{\"name\":\"payment\"},\"message\":\"provider latency ${i}42ms\",\"trace\":{\"id\":\"trc-${i}002\"}}"
  emit "{\"@timestamp\":\"2026-07-04T10:${minute}:02Z\",\"log\":{\"level\":\"WARN\"},\"kubernetes\":{\"namespace\":\"shop\",\"pod\":\"${checkout_pod}\"},\"service\":{\"name\":\"checkout\"},\"message\":\"retrying payment provider\",\"trace\":{\"id\":\"trc-${i}003\"}}"
  emit "{\"@timestamp\":\"2026-07-04T10:${minute}:03Z\",\"log\":{\"level\":\"ERROR\"},\"kubernetes\":{\"namespace\":\"shop\",\"pod\":\"${checkout_pod}\"},\"service\":{\"name\":\"checkout\"},\"message\":\"payment declined\",\"trace\":{\"id\":\"trc-${i}004\"}}"
  emit "{\"@timestamp\":\"2026-07-04T10:${minute}:04Z\",\"log\":{\"level\":\"INFO\"},\"kubernetes\":{\"namespace\":\"shop\",\"pod\":\"${cart_pod}\"},\"service\":{\"name\":\"cart\"},\"message\":\"cart updated\",\"trace\":{\"id\":\"trc-${i}005\"}}"
  emit "{\"@timestamp\":\"2026-07-04T10:${minute}:05Z\",\"log\":{\"level\":\"ERROR\"},\"kubernetes\":{\"namespace\":\"shop\",\"pod\":\"${worker_pod}\"},\"service\":{\"name\":\"payment\"},\"message\":\"fallback charge failed\",\"trace\":{\"id\":\"trc-${i}006\"}}"
done
