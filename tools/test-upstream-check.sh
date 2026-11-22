#!/usr/bin/env bash
# Tests for tools/upstream-check.sh against throw-away local repositories (no network).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
fail() { echo "FAIL: $*" >&2; exit 1; }

up="$T/up"; git init -q -b main "$up"
mkdir -p "$up/lib/common" "$up/lib/other"
echo 1 > "$up/lib/common/ns.c"; echo 1 > "$up/lib/other/x.c"; git -C "$up" add -A; git -C "$up" commit -qm base
BASE=$(git -C "$up" rev-parse HEAD)
printf 'REPOSITORY=unused\nBRANCH=main\nTAG=t\nCOMMIT=%s\n' "$BASE" > "$T/UPSTREAM"
printf '# comment\nlib/common/ns.c\tcrates/x.rs\twhat\tH\n\n# lib/other/x.c not monitored\n' > "$T/PATHS"
run() { UPSTREAM_FILE="$T/UPSTREAM" UPSTREAM_PATHS_FILE="$T/PATHS" UPSTREAM_REPOSITORY="$up" REPORT="$T/report.md" GITHUB_OUTPUT="$T/out" "$ROOT/tools/upstream-check.sh" >/dev/null 2>"$T/err"; }
out() { sed -n "s/^$1=//p" "$T/out" | tail -n1; }

# 1. nothing new
: > "$T/out"; run || fail "no change must succeed"
[ "$(out relevant)" = false ] && [ "$(out new_commits)" = 0 ] || fail "no change: $(cat "$T/out")"

# 2. an unrelated file changes: counted as new, not relevant
echo 2 > "$up/lib/other/x.c"; git -C "$up" commit -qam unrelated
: > "$T/out"; run || fail "unrelated change must succeed"
[ "$(out relevant)" = false ] && [ "$(out new_commits)" = 1 ] && [ "$(out relevant_commits)" = 0 ] || fail "unrelated: $(cat "$T/out")"
grep -q "No review needed" "$T/report.md" || fail "report should say no review needed"

# 3. a monitored file changes: relevant, with a fingerprint and the file in the report
echo 2 > "$up/lib/common/ns.c"; git -C "$up" commit -qam relevant
FP=$(git -C "$up" rev-parse HEAD)
: > "$T/out"; run || fail "relevant change must succeed"
[ "$(out relevant)" = true ] && [ "$(out relevant_commits)" = 1 ] && [ "$(out fingerprint)" = "$FP" ] || fail "relevant: $(cat "$T/out")"
grep -q 'lib/common/ns.c' "$T/report.md" && grep -q 'may need review' "$T/report.md" || fail "report should list the file"
grep -q 'lib/other/x.c' "$T/report.md" && fail "an unmonitored file must not be listed"

# 4. the baseline is not in upstream (rewritten history / wrong id): must fail, never "no changes"
printf 'REPOSITORY=unused\nBRANCH=main\nCOMMIT=%s\n' 0123456789abcdef0123456789abcdef01234567 > "$T/UPSTREAM"
run && fail "a missing baseline must fail"; grep -q "baseline" "$T/err" || fail "the error should name the baseline"

# 5. the baseline exists but is not an ancestor of the branch (another branch)
git -C "$up" checkout -q -b side "$BASE"; echo s > "$up/side.txt"; git -C "$up" add -A; git -C "$up" commit -qm side
SIDE=$(git -C "$up" rev-parse HEAD); git -C "$up" checkout -q main
printf 'REPOSITORY=unused\nBRANCH=main\nCOMMIT=%s\n' "$SIDE" > "$T/UPSTREAM"
run && fail "a baseline off the branch must fail" || true

# 6. the repository cannot be fetched
printf 'REPOSITORY=unused\nBRANCH=main\nCOMMIT=%s\n' "$BASE" > "$T/UPSTREAM"
UPSTREAM_FILE="$T/UPSTREAM" UPSTREAM_PATHS_FILE="$T/PATHS" UPSTREAM_REPOSITORY="$T/nowhere" REPORT="$T/r2.md" "$ROOT/tools/upstream-check.sh" >/dev/null 2>&1 && fail "a failed fetch must fail"

# 7. no paths configured
printf '# nothing\n' > "$T/PATHS"; run && fail "an empty path list must fail"
echo "upstream-check tests: all passed"
