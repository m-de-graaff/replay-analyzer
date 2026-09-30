#!/usr/bin/env bash
# Times replay-analyzer against other Siege replay parsers with hyperfine, on
# the Y11S3 replays in test_recordings/valid/Y11S3.
#
#   cargo build --release
#   R6_DISSECT=path/to/r6-dissect REPLAY_TOOL=path/to/replay-tool benches/compare.sh
#
# Each tool is optional: one that is not found is left out. Results go to
# target/compare/*.md.
#
#   r6-dissect   https://github.com/redraskal/r6-dissect (Go)
#   replay-tool  https://github.com/wnc-replay/replay-tool (Go)

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/test_recordings/valid/Y11S3"
round="$dir/custom_1.rec"
out="$root/target/compare"
rm -rf "$out"
mkdir -p "$out"

ra="${REPLAY_ANALYZER:-$root/target/release/replay-analyzer}"
r6d="${R6_DISSECT:-$(command -v r6-dissect || true)}"
rt="${REPLAY_TOOL:-$(command -v replay-tool || true)}"

have() { [ -n "$1" ] && [ -x "$1" ]; }
have "$ra" || { echo "build replay-analyzer first: cargo build --release" >&2; exit 1; }
have "$r6d" || echo "r6-dissect not found, skipping it (set R6_DISSECT)" >&2
have "$rt" || echo "replay-tool not found, skipping it (set REPLAY_TOOL)" >&2

# hyperfine runs commands without a shell (-N), so Git Bash paths must become
# native ones on Windows.
native() { if command -v cygpath >/dev/null; then cygpath -m "$1"; else echo "$1"; fi; }

# Which rounds each tool reads without failing. replay-tool is left out: it
# takes most of a minute a round.
echo "== compatibility"
ok_both=()
for f in "$dir"/*.rec; do
    line="$(basename "$f"):"
    if "$ra" "$(native "$f")" -o "$(native "$out/tmp.json")" 2>/dev/null; then
        line+=" replay-analyzer ok"
    else
        line+=" replay-analyzer FAIL"
    fi
    if have "$r6d"; then
        if "$r6d" "$(native "$f")" -o "$(native "$out/tmp.json")" >/dev/null 2>&1; then
            line+=", r6-dissect ok"
            ok_both+=("$f")
        else
            line+=", r6-dissect FAIL"
        fi
    fi
    echo "$line" | tee -a "$out/compat.txt"
done

# The match bench uses only rounds every tool can read.
subset="$out/match"
mkdir -p "$subset"
if have "$r6d"; then
    for f in "${ok_both[@]}"; do cp "$f" "$subset/"; done
else
    cp "$dir"/*.rec "$subset/"
fi
rounds=$(ls "$subset" | wc -l)

for v in dir round out subset ra r6d rt; do
    [ -n "${!v}" ] && printf -v "$v" '%s' "$(native "${!v}")"
done

hf() {
    local name="$1"; shift
    hyperfine -N --warmup 3 --export-markdown "$out/$name.md" "$@"
}

# One round to JSON: everything each tool extracts.
args=(-n replay-analyzer "'$ra' '$round' -o '$out/ra.json'")
have "$r6d" && args+=(-n r6-dissect "'$r6d' '$round' -o '$out/r6d.json'")
hf round "${args[@]}"

# Header and player list only.
args=(-n replay-analyzer "'$ra' --info '$round'")
have "$r6d" && args+=(-n r6-dissect "'$r6d' --info '$round'")
hf header "${args[@]}"

# A match folder to one JSON file.
echo "== match folder: $rounds rounds"
args=(-n replay-analyzer "'$ra' '$subset' -o '$out/ra-match.json'")
have "$r6d" && args+=(-n r6-dissect "'$r6d' '$subset' -o '$out/r6d-match.json'")
hf match "${args[@]}"

# replay-tool also reconstructs positions, aim and shots, and does so even for
# -header, so each run takes most of a minute. Three runs each.
if have "$rt"; then
    hyperfine -N --runs 3 --export-markdown "$out/replay-tool.md" \
        -n "replay-tool -header" "'$rt' -header '$round'" \
        -n "replay-tool" "'$rt' -o '$out/rt.json' '$round'"
fi
