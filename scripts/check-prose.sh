#!/usr/bin/env bash
# Flags padding and sales words in docs, Rust comments and commit messages (docs/STYLE.md).
#
# With no arguments it checks every tracked Markdown file and every Rust line with a comment on
# it. With arguments (the commit-msg hook passes the message file) it checks those files as
# commit messages. A line containing `prose:allow` is skipped. In Markdown, code blocks, inline
# code and URLs are ignored.
#
# Known gap: a phrase split across a line break ("in order" / "to") isn't caught.
#
# Usage: scripts/check-prose.sh [file...]
set -euo pipefail

# Keep in sync with "Words to avoid" in docs/STYLE.md. Matched case-insensitively as whole words.
words=(
  robust seamless seamlessly powerful cutting-edge state-of-the-art best-in-class world-class
  "blazing fast" "blazingly fast" game-changer elegant empower holistic synergy
  leverage leverages leveraging utilize utilizes utilizing facilitate facilitates streamline
  delve delves
  "it('|’)s important to note" "it is important to note"
  "it('|’)s worth noting" "it is worth noting"
  "needless to say" "in order to" basically essentially simply "a wide range of" plethora myriad
  tapestry
  furthermore moreover "in conclusion" "in summary"
)
# Spaces in a phrase match any run of whitespace. `('|’)` matches either apostrophe.
pattern=$(IFS='|'; echo "${words[*]}")
pattern=${pattern// /[[:space:]]+}

# The style guide lists the words on purpose, and this script defines them.
skip='^(docs/STYLE\.md|scripts/check-prose\.sh)$'
# A Rust line with a `//` or `/*` comment, or the `*` continuation of a block comment. Code on the
# same line gets checked too, which is fine: our identifiers don't use these words.
rust_comment='//|/\*|^\s*\*'

# Blanks fenced code blocks, inline code and URLs in Markdown, keeping line numbers.
strip_markdown_code() {
  awk '/^[[:space:]]*```/ { fenced = !fenced; print ""; next }
       fenced { print ""; next }
       { gsub(/`[^`]*`/, ""); gsub(/https?:\/\/[^ )>]*/, ""); print }'
}

# Reads text on stdin and prints "label:line:text" for each hit on lines matching $2.
# Each grep exits 1 when nothing matches, which isn't an error here.
check() {
  local label=$1 lines=$2
  { grep -nE -- "$lines" || true; } | { grep -v 'prose:allow' || true; } |
    { grep -Eiw -- "$pattern" || true; } | awk -v label="$label" '{ print label ":" $0 }'
}

scan() {
  if [ "$#" -gt 0 ]; then
    for file in "$@"; do
      [ -r "$file" ] || { echo "prose: can't read $file" >&2; return 2; }
      # Lines starting with '#' are git's template, and `git commit -v` puts the diff after
      # the scissors line, so neither is part of the message.
      sed '/^# -* >8 -*$/,$d' "$file" | check "$file" '^[^#]'
    done
  else
    # From the root, so `git ls-files` sees the whole repo.
    cd "$(git rev-parse --show-toplevel)"
    git ls-files '*.md' '*.rs' | while IFS= read -r file; do
      [[ $file =~ $skip ]] && continue
      case $file in
        *.rs) check "$file" "$rust_comment" < "$file" ;;
        *) strip_markdown_code < "$file" | check "$file" . ;;
      esac
    done
  fi
}

hits=$(scan "$@")
if [ -n "$hits" ]; then
  echo "prose: these lines use words docs/STYLE.md asks us to avoid. Rewrite them, or add" >&2
  echo "prose:allow to the line if the word is correct:" >&2
  echo "$hits" >&2
  exit 1
fi
echo "prose: ok"
