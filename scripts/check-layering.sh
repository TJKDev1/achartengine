#!/usr/bin/env bash
# Checks crate layering (DESIGN §4, §11): a crate may only depend on the workspace crates listed
# for it below, so lower layers can never reach up into higher ones. Engine crates also may not
# pull in windowing, GPU, WASM-runtime or Python crates.
#
# Every workspace crate must have an entry. When you add a crate, add its line here; the check
# fails until you do, so the layering is decided on purpose rather than by accident.
# Only normal and build dependencies count; dev-dependencies may reach up for tests.
#
# Usage: scripts/check-layering.sh   (needs cargo and jq)
set -euo pipefail

# crate -> space-separated workspace crates it may depend on.
declare -A allowed=(
  [ace-core]="ace-draw"
)

# crate -> space-separated external crates it must never depend on directly.
declare -A forbidden=(
  [ace-core]="wgpu winit wasmtime extism pyo3 tokio"
)

meta=$(cargo metadata --format-version 1 --no-deps --locked)
# jq on Windows prints CRLF; strip the CR so names compare cleanly.
names() { jq -r "$@" <<<"$meta" | tr -d '\r' | sort -u; }
members=$(names '.packages[].name')

status=0
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
    elif [[ " ${forbidden[$crate]:-} " == *" $dep "* ]]; then
      echo "layering: $crate must not depend on $dep" >&2
      status=1
    fi
  done
done

[ "$status" -eq 0 ] && echo "layering: ok ($(wc -w <<<"$members") crates)"
exit "$status"
