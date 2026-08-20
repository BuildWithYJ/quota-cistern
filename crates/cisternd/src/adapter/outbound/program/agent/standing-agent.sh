#!/bin/sh
#
# An agent that is not the vendor's, for this adapter's tests.
#
# It takes the arguments the adapter passes, ignores all of them but the
# instruction, and runs that instruction as a shell command. A program that
# refused the arguments would fail every one of those tests for the same
# reason and prove nothing about any of them.
#
# The instruction arrives as a system prompt and the goal as the prompt. Both
# are written out where a test can read them: {prompt} for the prompt and
# {prompt}.system for the instruction, filled the way claude.toml's places
# are, since a test cannot see the arguments a child was given any other way.

asked=
told=
while [ $# -gt 0 ]; do
  case "$1" in
  -p)
    shift
    asked="$1"
    ;;
  --append-system-prompt)
    shift
    told="$1"
    ;;
  esac
  shift
done

printf '%s' "$asked" > '{prompt}'
printf '%s' "$told" > '{prompt}.system'

[ -n "$told" ] || exit 0
exec /bin/sh -c "$(printf '%s' "$told" | tail -n 1)"
