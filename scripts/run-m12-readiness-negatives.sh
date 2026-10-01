#!/bin/sh
set -eu

# The helper uses inherited process-local faults on the actual native backend.
# It neither needs elevated privileges nor changes host security settings.
exec node scripts/run-python.mjs scripts/m12_native.py readiness
