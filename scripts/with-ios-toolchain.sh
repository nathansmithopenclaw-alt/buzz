#!/usr/bin/env bash
set -euo pipefail

# Flutter native-asset hooks retain PATH but remove DEVELOPER_DIR. Keep the
# explicitly selected Xcode available to their xcrun calls without changing
# xcode-select globally or modifying cached Dart dependencies.
toolchain_dir="${DEVELOPER_DIR:-$(/usr/bin/xcode-select -p)}"
if [[ ! -x "$toolchain_dir/usr/bin/xcodebuild" ]]; then
    echo "Select a full Xcode installation with DEVELOPER_DIR before building iOS." >&2
    exit 2
fi
if [[ $# -eq 0 ]]; then
    echo "Usage: DEVELOPER_DIR=/path/to/Xcode.app/Contents/Developer $0 <command> [args...]" >&2
    exit 2
fi
toolchain_bin="$(mktemp -d "${TMPDIR:-/tmp}/buzz-ios-toolchain.XXXXXX")"
trap 'rm -rf -- "$toolchain_bin"' EXIT
{
    printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail'
    printf 'export DEVELOPER_DIR=%q\n' "$toolchain_dir"
    printf '%s\n' 'exec /usr/bin/xcrun "$@"'
} > "$toolchain_bin/xcrun"
chmod 700 "$toolchain_bin/xcrun"
export DEVELOPER_DIR="$toolchain_dir"
export PATH="$toolchain_bin:$PATH"
"$@"
