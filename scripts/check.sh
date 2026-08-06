#!/usr/bin/env bash
#
# Every check a machine can make. CI calls this same script.
#
# Steps run to the end even after one fails, so a single run reports all of them.
# Output streams; the banners mark where to scroll back to.

set -u
cd "$(dirname "$0")/.." || exit 1

readonly WIDTH=64
readonly LABEL=16

failed=0
failed_steps=""
summary=""

rule() {
  printf '%*s\n' "$WIDTH" '' | tr ' ' '='
}

banner() {
  local label=" $1 "
  printf '\n===%s' "$label"
  printf '%*s\n' "$((WIDTH - ${#label} - 3))" '' | tr ' ' '='
}

step() {
  local name="$1"
  local result
  shift
  banner "$name"
  if "$@"; then
    result="passed"
  else
    result="failed"
    failed=1
    failed_steps="${failed_steps:+$failed_steps, }$name"
  fi
  summary="${summary}$(printf '%-*s%s' "$LABEL" "$name" "$result")"$'\n'
}

# Markdown is exempt: prose and the translations need more than ASCII.
#
# The list comes from the working tree, not from git, so an uncommitted file is still checked.
sources() {
  find . -type f \
    \( -name '*.rs' -o -name '*.toml' -o -name '*.sh' \
       -o -name '*.yml' -o -name '*.yaml' \) \
    -not -path './target/*' -not -path './.private/*' -not -path './.git/*'
}

# Comments and documentation are written in English.
# This is the part of that rule a machine can decide.
ascii_only() {
  local files count hits
  files=$(sources)
  count=$(printf '%s' "$files" | grep -c '' || true)

  printf 'scanning %s files\n' "$count"
  if [ "$count" -eq 0 ]; then
    printf 'no files matched, so nothing was checked\n'
    return 1
  fi

  # -CSD reports a codepoint rather than a byte.
  # close ARGV restarts $. at each file.
  hits=$(printf '%s\n' "$files" | xargs perl -CSD -ne '
    my @bad = /([^\x09\x0A\x20-\x7E])/g;
    if (@bad) {
      my $points = join(" ", map { sprintf "U+%04X", ord } @bad);
      printf "%s:%d: %s: %s", $ARGV, $., $points, $_;
    }
    close ARGV if eof;
  ')
  if [ -n "$hits" ]; then
    printf '%s\n' "$hits"
    return 1
  fi
  return 0
}

step "fmt" cargo fmt --check
step "lint" cargo clippy -q --all-targets --all-features -- -D warnings
step "test" cargo test -q
step "ascii" ascii_only

printf '\n'
rule
printf '%s\n' "$summary"
if [ "$failed" -eq 0 ]; then
  printf 'all checks passed\n'
else
  printf 'failed: %s\n' "$failed_steps"
fi
exit "$failed"
