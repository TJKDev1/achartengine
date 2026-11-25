#!/usr/bin/env bash
# PostToolUse(Edit|Write): rustfmt the file Claude just edited, if it is a .rs file.
f=$(jq -r '.tool_response.filePath // .tool_input.file_path // empty')
case "$f" in
  *.rs) [ -f "$f" ] && rustfmt --edition 2024 "$f" ;;
esac
exit 0
