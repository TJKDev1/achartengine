#!/usr/bin/env bash
# Checks crate layering (DESIGN §4, §11): a crate may only depend on the workspace crates listed
# for it below, so lower layers can never reach up into higher ones. Engine crates also may not
# pull in windowing, GPU, WASM-runtime or Python crates, directly or through another crate.
# Only normal and build dependencies count; dev-dependencies may reach up for tests.
#
# Every workspace crate must have an entry. When you add a crate, add its line here; the check
# fails until you do, so the layering is decided on purpose rather than by accident.
#
# Usage: scripts/check-layering.sh   (needs cargo and jq)
set -euo pipefail

# crate -> space-separated workspace crates it may depend on.
declare -A allowed=(
  [ace-core]="ace-draw"
  [ace-host]="ace-core"
  [ace-host-wasmtime]="ace-host ace-core"
)

# crate -> space-separated patterns that no crate in its resolved dependency tree may match.
# wasmi builds on wasm32v1-none, so only this check would catch it (ADR 0005).
# ace-host-wasmtime has no entry: it is the crate that brings in wasmtime.
engine_forbidden="wgpu* winit* wasmtime* wasmi* extism* pyo3* tokio*"
declare -A forbidden=(
  [ace-core]="$engine_forbidden"
  [ace-host]="ace-host-wasmtime $engine_forbidden"
)

meta=$(cargo metadata --format-version 1 --no-deps --locked)
# jq on Windows prints CRLF; strip the CR so names compare cleanly.
names() { jq -r "$@" <<<"$meta" | tr -d '\r' | sort -u; }
members=$(names '.packages[].name')

status=0

# Fails if any crate in the normal and build dependency tree of $1 matches a forbidden pattern.
check_tree() {
  local crate=$1 dep pattern patterns tree
  [[ -v "forbidden[$crate]" ]] || return 0
  read -ra patterns <<<"${forbidden[$crate]}"
  # Assigned on its own line so a failing `cargo tree` stops the script instead of looping
  # over nothing.
  tree=$(cargo tree -p "$crate" -e normal,build --prefix none --format '{p}' --locked)
  for dep in $(cut -d' ' -f1 <<<"$tree" | tr -d '\r' | sort -u); do
    for pattern in "${patterns[@]}"; do
      # shellcheck disable=SC2053 # the pattern is meant to glob
      if [[ $dep == $pattern ]]; then
        echo "layering: $crate must not depend on $dep, even indirectly" >&2
        status=1
      fi
    done
  done
}

for crate in $members; do
  if [[ ! -v "allowed[$crate]" ]]; then
    echo "layering: $crate has no entry in scripts/check-layering.sh; decide what it may depend on" >&2
    status=1
    continue
  fi
  deps=$(jq -r --arg c "$crate" \
    '.packages[] | select(.name == $c) | .dependencies[] | select(.kind != "dev") | .name' <<<"$meta" | sort -u)
  for dep in $deps; do
    if grep -qx "$dep" <<<"$members"; then
      if [[ " ${allowed[$crate]} " != *" $dep "* ]]; then
        echo "layering: $crate must not depend on $dep (allowed: ${allowed[$crate]:-none})" >&2
        status=1
      fi
    fi
  done
  check_tree "$crate"
done

[ "$status" -eq 0 ] && echo "layering: ok ($(wc -w <<<"$members") crates)"
exit "$status"
