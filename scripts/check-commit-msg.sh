#!/usr/bin/env sh
# Checks that the first line of a commit message follows Conventional Commits.
# Usage: check-commit-msg.sh <path-to-commit-msg-file>
set -eu

msg_file="$1"
subject=$(head -n 1 "$msg_file" | tr -d '\r')

# Let git's own generated messages through.
case "$subject" in
  "Merge "* | "Revert \""* | "fixup! "* | "squash! "*) exit 0 ;;
esac

types='feat|fix|perf|refactor|docs|test|build|ci|chore|style|revert'
pattern="^($types)(\([a-z0-9][a-z0-9-]*\))?!?: [^ ].*$"

if ! printf '%s\n' "$subject" | grep -Eq "$pattern"; then
  echo "commit-msg: subject is not a Conventional Commit:" >&2
  echo "  $subject" >&2
  echo "expected: <type>(<optional-scope>)!?: <description>" >&2
  echo "types:    $types" >&2
  echo "example:  feat(core): add column store" >&2
  exit 1
fi

if [ "${#subject}" -gt 72 ]; then
  echo "commit-msg: subject is ${#subject} chars; keep it at 72 or fewer." >&2
  exit 1
fi
