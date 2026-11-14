#!/bin/sh
# hostile.sh <bin> <dir>: each hostile diagram through <bin>, 60 s at most: its CPU time, its peak
# RSS and what it said. macOS's /usr/bin/time -l.
bin=$1; dir=$2; out=$(mktemp -d)
for f in "$dir"/*.mmd; do
  /usr/bin/time -l timeout 60 "$bin" "$f" "$out/x.svg" > "$out/o" 2> "$out/e"; code=$?
  user=$(awk '/ user /{print $3}' "$out/e"); rss=$(awk '/maximum resident set size/{printf "%.0f", $1/1048576}' "$out/e")
  said=$(grep -v -E 'real|resident|page|swap|block|messages|signals|context|instructions|cycles|footprint|^ *[0-9]+ +[a-z]' "$out/e" | head -1 | cut -c1-90)
  printf '%-24s exit %-3s user %6ss  rss %5s MiB  %s\n' "$(basename $f)" "$code" "$user" "$rss" "$said"
done
