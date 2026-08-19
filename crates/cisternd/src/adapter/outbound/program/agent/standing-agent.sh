#!/bin/sh
#
# An agent that is not the vendor's, for this adapter's tests.
#
# It takes the arguments the adapter passes, ignores all of them but the
# instruction, and runs that instruction as a shell command. A program that
# refused the arguments would fail every one of those tests for the same
# reason and prove nothing about any of them.
#
# The prompt leads with the goal and the instruction follows a blank line, so
# the last line is what a test asked for. Where the prompt is written is
# {prompt}, filled the way claude.json's places are, since a test cannot see
# the arguments a child was given any other way. Every argument is written
# beside it, one to a line, for the tests that are about those.

printf '%s\n' "$@" > '{prompt}.args'

while [ $# -gt 0 ]; do
  if [ "$1" = -p ]; then
    shift
    printf '%s' "$1" > '{prompt}'
    exec /bin/sh -c "$(printf '%s' "$1" | tail -n 1)"
  fi
  shift
done
exit 0
