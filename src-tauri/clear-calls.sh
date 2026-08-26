#!/usr/bin/env bash
# Wipe every raven call and its turns from the live companion database.
#
# Test-loop tool: a call is capped at 5 messages and a companion at 5 calls a
# day, so a few test rings exhaust the allowance. This resets the board.
#
# The database is at ONE fixed path, anchored to the real $HOME.
#
# It used to follow XDG_DATA_HOME, which VS Code's snap redirects to
# ~/snap/code/<REVISION>/.local/share — so the app had a DIFFERENT database
# depending on where it was launched from AND on which VS Code update was
# installed. Now it lives beside the rest of Semantix's global state; see
# resolve_database_path in src/lib.rs.
set -euo pipefail

db="${COMPANION_DB:-$HOME/.semantix/companion/companion.db}"

if [ ! -f "$db" ]; then
  echo "no companion database at $db" >&2
  echo "(has the app been launched since the ~/.semantix move? it migrates on first start)" >&2
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
