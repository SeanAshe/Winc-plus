#!/bin/sh
# On each commit, store one more than the fork version already in HEAD.
# Repeating this before the commit succeeds writes the same number again.
root=$(git rev-parse --show-toplevel) || exit 0
file="$root/release/fork-version"
current=$(git show HEAD:release/fork-version 2>/dev/null) || exit 0
current=$(printf '%s' "$current" | tr -d '[:space:]')
case "$current" in
  ''|*[!0-9]*) exit 0 ;;
esac
printf '%s\n' "$((current + 1))" > "$file"
git add -- "$file"
