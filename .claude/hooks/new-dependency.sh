#!/usr/bin/env bash
# PreToolUse(Edit|Write): if an edit to a Cargo.toml adds an external dependency, ask the user
# before it goes through. Backs the CLAUDE.md rule "don't add a dependency without asking".
# Our own `ace-*` crates don't count; crate layering is checked by scripts/check-layering.sh.
input=$(cat)
file=$(jq -r '.tool_input.file_path // empty' <<<"$input" | tr -d '\r')
case "$file" in
  *Cargo.toml) ;;
  *) exit 0 ;;
esac

before=""
[ -f "$file" ] && before=$(cat "$file")

# The file as it will look after the edit.
after=$(jq -r --arg cur "$before" '
  if .tool_name == "Write" then .tool_input.content
  elif .tool_input.replace_all then $cur | split(.tool_input.old_string) | join(.tool_input.new_string)
  else (.tool_input.old_string) as $old
    | ($cur | index($old)) as $i
    | if $i == null then $cur
      else $cur[:$i] + .tool_input.new_string + $cur[($i + ($old | length)):]
      end
  end' <<<"$input")

# Dependency names in any [*dependencies*] table, plus [*dependencies.<name>] tables.
deps() {
  tr -d '\r' | awk '
    /^[[:space:]]*\[/ {
      in_deps = 0
      if (match($0, /dependencies\.[A-Za-z0-9_-]+\]/)) {
        name = substr($0, RSTART + 13, RLENGTH - 14); print name; next
      }
      if ($0 ~ /dependencies\][[:space:]]*(#.*)?$/) in_deps = 1
      next
    }
    in_deps && match($0, /^[[:space:]]*[A-Za-z0-9_-]+[[:space:]]*(\.|=)/) {
      name = $0; sub(/^[[:space:]]*/, "", name); sub(/[[:space:]]*(\.|=).*/, "", name); print name
    }' | grep -v '^ace-' | sort -u
}

added=$(comm -13 <(deps <<<"$before") <(deps <<<"$after") | tr '\n' ' ' | sed 's/ $//')
[ -z "$added" ] && exit 0

reason="This edit adds dependencies to $(basename "$(dirname "$file")")/Cargo.toml: $added. CLAUDE.md: don't add a dependency without asking the user first."
jq -n --arg r "$reason" '{hookSpecificOutput: {hookEventName: "PreToolUse", permissionDecision: "ask", permissionDecisionReason: $r}}'
exit 0
