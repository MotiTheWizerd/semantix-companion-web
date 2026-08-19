#!/usr/bin/env bash

# VS Code's Snap package injects GTK and library paths from its own runtime.
# Native Tauri windows must use the host system libraries instead.
set -e

cd "$(dirname "$0")"

command_name="${1:-dev}"
if [ "$#" -gt 0 ]; then
  shift
fi

env -u GTK_PATH \
    -u GIO_MODULE_DIR \
    -u GDK_PIXBUF_MODULE_FILE \
    -u GSETTINGS_SCHEMA_DIR \
    -u LOCPATH \
    -u LD_LIBRARY_PATH \
    -u GIO_LAUNCHED_DESKTOP_FILE \
    npm run tauri -- "$command_name" "$@"
