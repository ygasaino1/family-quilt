#!/usr/bin/env bash
# Compare Graphviz upstream with the baseline in UPSTREAM, looking only at the paths in UPSTREAM_PATHS.
#
# Writes a Markdown report (REPORT, default upstream-report.md) and, under GitHub Actions, the step summary and
# step outputs (relevant, fingerprint, new_commits, relevant_commits). Exit status: 0 = the check ran (changes
# or not), anything else = the check could NOT run (fetch failed, baseline missing, history rewritten, bad
# config). A failure is never reported as "no changes". Nothing is copied into this repository and nothing in
# it is modified except the report file.
#
# Test hooks: UPSTREAM_FILE, UPSTREAM_PATHS_FILE, UPSTREAM_REPOSITORY (a path or URL replacing REPOSITORY).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
UPSTREAM_FILE=${UPSTREAM_FILE:-$ROOT/UPSTREAM}
PATHS_FILE=${UPSTREAM_PATHS_FILE:-$ROOT/UPSTREAM_PATHS}
REPORT=${REPORT:-$ROOT/upstream-report.md}
WEB=${UPSTREAM_WEB:-https://gitlab.com/graphviz/graphviz}

die() { echo "upstream-check: $*" >&2; exit 2; }

[ -f "$UPSTREAM_FILE" ] || die "missing $UPSTREAM_FILE"
[ -f "$PATHS_FILE" ] || die "missing $PATHS_FILE"
get() { sed -n "s/^$1=//p" "$UPSTREAM_FILE" | head -n1; }
REPOSITORY=${UPSTREAM_REPOSITORY:-$(get REPOSITORY)}
BRANCH=$(get BRANCH)
TAG=$(get TAG)
COMMIT=$(get COMMIT)
[ -n "$REPOSITORY" ] && [ -n "$BRANCH" ] && [ -n "$COMMIT" ] || die "UPSTREAM needs REPOSITORY, BRANCH and COMMIT"
case "$COMMIT" in *[!0-9a-f]*|"") die "COMMIT is not a hex id: $COMMIT" ;; esac

# The verified paths: first column of every non-comment, non-empty line.
mapfile -t PATHS < <(grep -v '^[[:space:]]*#' "$PATHS_FILE" | grep -v '^[[:space:]]*$' | cut -f1)
[ "${#PATHS[@]}" -gt 0 ] || die "UPSTREAM_PATHS lists no paths"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
# History only (commits and trees); no file contents are downloaded and none are kept.
git clone --quiet --filter=blob:none --no-checkout --single-branch --branch "$BRANCH" "$REPOSITORY" "$WORK/up" \
  || die "cannot fetch $REPOSITORY ($BRANCH)"
cd "$WORK/up"
LATEST=$(git rev-parse "origin/$BRANCH^{commit}") || die "cannot resolve origin/$BRANCH"
BASE=$(git rev-parse --verify --quiet "$COMMIT^{commit}") \
  || die "baseline $COMMIT is not in the upstream history of $BRANCH (missing or history rewritten)"
git merge-base --is-ancestor "$BASE" "$LATEST" \
  || die "baseline $BASE is not an ancestor of $LATEST: upstream history was rewritten or the baseline is on another branch"

NEW=$(git rev-list --count "$BASE..$LATEST")
RELEVANT=$(git rev-list --count "$BASE..$LATEST" -- "${PATHS[@]}")
FINGERPRINT=none
[ "$RELEVANT" -gt 0 ] && FINGERPRINT=$(git rev-list -n1 "$LATEST" -- "${PATHS[@]}")

{
  echo "# Graphviz upstream check"
  echo
  echo "| | |"
  echo "|---|---|"
  echo "| Baseline | \`$BASE\`${TAG:+ (release $TAG)} |"
  echo "| Latest upstream | [\`${LATEST:0:12}\`]($WEB/-/commit/$LATEST) on \`$BRANCH\` |"
  echo "| New upstream commits since the baseline | $NEW |"
  echo "| Of those, touching the monitored files | $RELEVANT |"
  echo
  if [ "$RELEVANT" -eq 0 ]; then
    echo "**No review needed.** None of the $NEW new upstream commits changed a file our Rust ports were made from."
  else
    echo "**The Rust port may need review.** An upstream change is a reason to look, not proof that the port is outdated; the recorded baseline does not move until we decide the port has caught up."
    echo
    echo "[All changes since the baseline]($WEB/-/compare/$BASE...$LATEST)"
    echo
    echo "## Monitored files that changed"
    echo
    echo "| File | Rust counterpart | Commits | Lines +/- | History |"
    echo "|---|---|---|---|---|"
    for p in "${PATHS[@]}"; do
      c=$(git rev-list --count "$BASE..$LATEST" -- "$p")
      [ "$c" -gt 0 ] || continue
      rust=$(grep -F "$p"$'\t' "$PATHS_FILE" | head -n1 | cut -f2)
      stat=$(git diff --numstat "$BASE" "$LATEST" -- "$p" | awk '{a+=$1; d+=$2} END {printf "+%d / -%d", a, d}')
      echo "| \`$p\` | \`$rust\` | $c | $stat | [commits]($WEB/-/commits/$BRANCH/$p) |"
    done
    echo
    echo "## Commits touching them (newest first)"
    echo
    git log --format="- [\`%h\`]($WEB/-/commit/%H) %s (%as)" "$BASE..$LATEST" -- "${PATHS[@]}"
  fi
} > "$REPORT"

cat "$REPORT"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$REPORT" >> "$GITHUB_STEP_SUMMARY"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "relevant=$([ "$RELEVANT" -gt 0 ] && echo true || echo false)"
    echo "fingerprint=$FINGERPRINT"
    echo "new_commits=$NEW"
    echo "relevant_commits=$RELEVANT"
    echo "report=$REPORT"
  } >> "$GITHUB_OUTPUT"
fi
