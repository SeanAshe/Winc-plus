#!/usr/bin/env bash
# Fails when a program needs a newer glibc than GLIBC_FLOOR, or links a library that is not part of
# the C library: the packages declare no other link-time dependency.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=release/linux/baseline.env
source "$here/baseline.env"
(($#)) || { echo "usage: verify-baseline.sh <program>..." >&2; exit 2; }
failed=0
for file in "$@"; do
  newest="$(objdump -T "$file" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -n 1)"
  [[ -n "$newest" ]] || { echo "::error::$file: no glibc symbols found"; failed=1; continue; }
  if [[ "$(printf '%s\n' "GLIBC_$GLIBC_FLOOR" "$newest" | sort -V | tail -n 1)" != "GLIBC_$GLIBC_FLOOR" ]]; then
    echo "::error::$file needs $newest, newer than glibc $GLIBC_FLOOR"
    failed=1
  fi
  while read -r library; do
    case "$library" in
      # Before glibc 2.34 these were separate libraries; all of them belong to the C library.
      libc.so.6 | libm.so.6 | libdl.so.2 | libpthread.so.0 | librt.so.1 | libutil.so.1) ;;
      libresolv.so.2 | libgcc_s.so.1 | ld-linux-*.so.*) ;;
      *) echo "::error::$file links $library"; failed=1 ;;
    esac
  done < <(readelf -d "$file" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')
  echo "$file: newest symbol version $newest"
done
exit "$failed"
