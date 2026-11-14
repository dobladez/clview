#!/bin/sh
# worker.sh <clv> <diagram>...: each diagram drawn by clv's own worker (`clv --worker mermaid`, its
# caps armed), given one request frame on stdin: its CPU time, its peak RSS and what it answered
# (kind 2 its SVG, 3 merman's refusal; a worker killed answers nothing). macOS's /usr/bin/time -l.
clv=$1; shift; out=$(mktemp -d)
# A little-endian u32's four bytes, for printf.
le() { printf '\\%03o\\%03o\\%03o\\%03o' $(($1 & 255)) $(($1 >> 8 & 255)) $(($1 >> 16 & 255)) $(($1 >> 24 & 255)); }
for f in "$@"; do
  size=$(wc -c < "$f" | tr -d ' ')
  { printf "\\001$(le 1)$(le "$size")"; cat "$f"; } > "$out/frame"
  /usr/bin/time -l "$clv" --worker mermaid < "$out/frame" > "$out/o" 2> "$out/e"; code=$?
  user=$(awk '/ user /{print $3}' "$out/e"); rss=$(awk '/maximum resident set size/{printf "%.0f", $1/1048576}' "$out/e")
  peak=$(awk '/peak memory footprint/{printf "%.0f", $1/1048576}' "$out/e")
  kind=$(head -c 1 "$out/o" | od -An -tu1 | tr -d ' ')
  printf '%-26s exit %-3s user %5ss  rss %4s MiB  peak %4s MiB  answer %s, %s bytes\n' "$(basename "$f")" "$code" "$user" "$rss" "$peak" "${kind:-none}" "$(wc -c < "$out/o" | tr -d ' ')"
done
rm -r "$out"
