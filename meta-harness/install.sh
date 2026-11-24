#!/bin/sh
set -eu

fail() {
    printf '%s\n' "meta-install: $1" >&2
    exit 1
}

usage() {
    printf '%s\n' \
        'Usage: install.sh [--prefix <absolute-prefix>] [--no-service]' \
        '' \
        'Install Meta-Harness from this unpacked download on macOS arm64 or Linux x86_64.' \
        '' \
        'Options:' \
        '  --prefix <absolute-prefix>  Install into this prefix (default: $HOME/.local/meta-harness).' \
        '  --no-service                Install the files without starting the login service.' \
        '  -h, --help                  Show this help.'
}

prefix=
service=1
while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --prefix)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail 'missing value for --prefix'
            [ -z "$prefix" ] || fail '--prefix may be supplied only once'
            prefix=$2
            shift 2
            ;;
        --no-service)
            service=0
            shift
            ;;
        *) fail "unknown argument: $1" ;;
    esac
done

if [ -z "$prefix" ]; then
    [ -n "${HOME-}" ] || fail 'HOME is unset; pass --prefix'
    prefix=$HOME/.local/meta-harness
fi
case "$prefix" in
    /*) ;;
    *) fail '--prefix must be absolute' ;;
esac
newline=$(printf '\n_')
newline=${newline%_}
case "$prefix" in
    *"$newline"*) fail '--prefix must not contain a newline' ;;
esac

case "$(uname -s):$(uname -m)" in
    Darwin:arm64) platform=darwin-arm64 ;;
    Linux:x86_64) platform=linux-x64 ;;
    *) fail "no Meta-Harness build for $(uname -s) $(uname -m)" ;;
esac

case "$0" in
    */*) directory=${0%/*} ;;
    *) directory=. ;;
esac
bundle=$(CDPATH= cd "${directory:-/}" && pwd) || fail 'cannot open the download folder'
[ -f "$bundle/bundle-info" ] || fail 'missing bundle-info; run install.sh from an unpacked download'
{
    IFS= read -r version && IFS= read -r target && ! IFS= read -r extra
} < "$bundle/bundle-info" || fail 'invalid bundle-info'
printf '%s\n' "$version" | grep -Eq '^[0-9]+[.][0-9]+[.][0-9]+([-][0-9A-Za-z.-]+)?([+][0-9A-Za-z.-]+)?$' \
    || fail 'invalid bundle version'
[ "$target" = "$platform" ] || fail "download is for $target, not $platform"

set -- "$bundle"/lib/meta-harness/t3-*-"$platform"
[ "$#" -eq 1 ] && [ -d "$1" ] || fail "download lacks one T3 Code folder for $platform"
t3_source=$1
t3_name=${t3_source##*/}
for relative in bin/meta "lib/meta-harness/$t3_name/t3" LICENSE LICENSE-t3code THIRD_PARTY_NOTICES.md; do
    [ -f "$bundle/$relative" ] || fail "download lacks $relative"
done

lib=$prefix/lib/meta-harness
bin=$prefix/bin
share=$prefix/share/meta-harness
data=${META_HARNESS_HOME:-${HOME-}/.meta-harness}
case "$data/" in
    "$prefix/" | "$lib/"* | "$share/"*)
        fail "the data folder $data lies in the installation, so nothing was installed"
        ;;
esac
t3_stage=$lib/.$t3_name.install.$$
meta_stage=$bin/.meta.install.$$
cleanup() {
    status=$?
    rm -rf -- "$t3_stage"
    rm -f -- "$meta_stage"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

mkdir -p -- "$lib" "$bin" "$share"
cp -R "$t3_source" "$t3_stage"
cp "$bundle/bin/meta" "$meta_stage"
chmod 755 "$meta_stage"

rm -rf -- "$lib/$t3_name"
mv -- "$t3_stage" "$lib/$t3_name"
mv -f -- "$meta_stage" "$bin/meta"
for old in "$lib"/t3-*; do
    if [ -d "$old" ] && [ "$old" != "$lib/$t3_name" ]; then
        rm -rf -- "$old"
    fi
done
cp -f "$bundle/LICENSE" "$bundle/LICENSE-t3code" "$bundle/THIRD_PARTY_NOTICES.md" "$share/"

if [ "$service" -eq 1 ] && ! "$bin/meta" service install; then
    fail "the files are installed in $prefix, but the login service did not start; \
--no-service installs without it"
fi
printf 'Installed Meta-Harness %s in %s\n' "$version" "$prefix"
case ":${PATH-}:" in
    *":$bin:"*) ;;
    *) printf 'Add it to PATH: export PATH="%s:$PATH"\n' "$bin" ;;
esac
