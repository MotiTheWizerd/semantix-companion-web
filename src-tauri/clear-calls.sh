#!/usr/bin/env bash
# Wipe every raven call and its turns from the live companion database.
#
# Test-loop tool: a call is capped at 5 messages and a companion at 5 calls a
# day, so a few test rings exhaust the allowance. This resets the board.
#
# ⚑ THE DATABASE LIVES WHEREVER XDG_DATA_HOME POINTS WHEN THE APP LAUNCHES.
# Started from VS Code's terminal, that is inside VS Code's snap sandbox, NOT
# ~/.local/share — so the app has a DIFFERENT database depending on where it
# was launched from. This resolves it the same way Tauri does rather than
# guessing, which is the only way to be sure you cleared the one in use.
set -euo pipefail

data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
db="$data_home/com.semantix.companion/companion.db"

if [ ! -f "$db" ]; then
  echo "no companion database at $db" >&2
  echo "(launched from a different shell? XDG_DATA_HOME is ${XDG_DATA_HOME:-<unset>})" >&2
  exit 1
fi

echo "clearing calls in $db"
sqlite3 "$db" <<'SQL'
PRAGMA foreign_keys=ON;
DELETE FROM raven_call_messages;
DELETE FROM raven_calls;
SELECT 'calls left:    ' || COUNT(*) FROM raven_calls;
SELECT 'messages left: ' || COUNT(*) FROM raven_call_messages;
SQL
