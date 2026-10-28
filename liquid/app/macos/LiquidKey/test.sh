#!/bin/sh
# Runs the Liquid key helper's checks; needs only the Swift compiler.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# Top-level checks must be the program's main.swift.
cp "$here/MappingTests.swift" "$work/main.swift"
xcrun swiftc -swift-version 5 -o "$work/checks" "$here/Mapping.swift" "$work/main.swift"
"$work/checks"
# The helper itself must compile too.
xcrun swiftc -swift-version 5 -typecheck "$here/Mapping.swift" "$here/main.swift"
