#!/bin/sh
set -eu

exec node scripts/run-python.mjs scripts/m12_native.py readiness
