#!/bin/sh
# The fork version is how many commits this commit is ahead of DEEIX-AI/Winc dev.
# The commit being created is not in HEAD yet, so it counts as one more.
root=$(git rev-parse --show-toplevel) || exit 0
file="$root/release/fork-version"
git fetch --quiet https://github.com/DEEIX-AI/Winc.git "+dev:refs/remotes/upstream/dev" || true
base=$(git rev-parse --verify --quiet refs/remotes/upstream/dev) || exit 0
if git rev-parse --verify --quiet MERGE_HEAD >/dev/null; then
  count=$(git rev-list --count HEAD MERGE_HEAD --not "$base") || exit 0
else
  count=$(git rev-list --count HEAD --not "$base") || exit 0
fi
count=$((count + 1))
case "$count" in
  ''|*[!0-9]*|0) exit 0 ;;
esac
printf '%s\n' "$count" > "$file"
git add -- "$file"
