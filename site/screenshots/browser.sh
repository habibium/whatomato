#!/usr/bin/env sh
# browser.sh <name> <agent-browser arguments>: one isolated browser per name, in the demo's time zone
set -eu
name=$1
shift
TZ=Europe/London exec agent-browser --session "wa-docs-$name" "$@"
