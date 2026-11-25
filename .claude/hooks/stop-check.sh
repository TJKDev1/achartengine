#!/usr/bin/env bash
# Stop: if Rust or TOML files changed, run clippy (-D warnings). On failure, exit 2 so the
# errors go back to Claude to fix. Runs only once per stop (stop_hook_active) to avoid loops.
input=$(cat)
active=$(printf '%s' "$input" | jq -r '.stop_hook_active // false')
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0
[ -f Cargo.toml ] || exit 0

# "Changed" means uncommitted (incl. untracked) OR committed but not yet pushed: Claude often
# commits before it stops, which leaves a clean tree. Pushed work already passed pre-push.
changed=$(git status --porcelain -- '*.rs' '*.toml' 2>/dev/null)
if [ -z "$changed" ]; then
  if git rev-parse --verify --quiet '@{upstream}' >/dev/null; then
    changed=$(git diff --name-only '@{upstream}' -- '*.rs' '*.toml' 2>/dev/null)
  else
    changed="no upstream"
  fi
fi
[ -z "$changed" ] && exit 0

out=$(cargo clippy --workspace --all-targets --all-features --quiet --message-format=short -- -D warnings 2>&1)
status=$?
[ $status -eq 0 ] && exit 0

if [ "$active" = "true" ]; then
  printf '{"systemMessage": "cargo clippy still fails; see `cargo clippy --workspace --all-targets`."}\n'
  exit 0
fi
{
  echo "cargo clippy failed (workspace lints are strict; fix these before finishing):"
  printf '%s\n' "$out" | grep -E '(error|warning)(\[|:)' | grep -v '^error: could not compile' | head -40
} >&2
exit 2
