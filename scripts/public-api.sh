#!/usr/bin/env bash
# Writes each workspace crate's public API to crates/<crate>/public-api.txt (DESIGN §6.5).
# The files are committed, so any public API change shows up as a diff in review. CI reruns
# this and fails if a file changed; rerun it locally and commit the result when that happens.
#
# Needs cargo-public-api and a nightly toolchain (for rustdoc JSON):
#   cargo install --locked cargo-public-api && rustup toolchain install nightly --profile minimal
set -euo pipefail

# Library crates as "name directory". jq on Windows prints CRLF; strip the CR.
crates=$(cargo metadata --format-version 1 --no-deps --locked | jq -r '
  .packages[] | select(any(.targets[]; .kind | index("lib")))
  | "\(.name) \(.manifest_path | rtrimstr("Cargo.toml"))"' | tr -d '\r')

while read -r crate dir; do
  # -s leaves out blanket impls (`impl<T> From<T> for T` etc.), which are noise.
  cargo public-api --package "$crate" -s | tr -d '\r' >"${dir}public-api.txt"
  echo "public-api: $crate"
done <<<"$crates"
