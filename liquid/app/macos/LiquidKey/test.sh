#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$here/MappingTests.swift" "$work/main.swift"
xcrun swiftc -swift-version 5 -o "$work/checks" "$here/Mapping.swift" "$work/main.swift"
"$work/checks"
xcrun swiftc -swift-version 5 -typecheck "$here/Mapping.swift" "$here/main.swift"
