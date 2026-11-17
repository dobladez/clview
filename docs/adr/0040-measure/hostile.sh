#!/bin/sh
# Each hostile kind at growing sizes, through `measure hostile`, each in a process of its own so
# that a stack overflow (SIGSEGV/SIGBUS, or SIGABRT from Rust's guard) costs that line only.
# ./hostile.sh <orgize|orgrust> [stack MiB] [binary]
parser=$1; stack=${2:-8}; bin=${3:-target/release/measure}
for kind in headlines list quotes specials drawers-nested emphasis emphasis-words unclosed unclosed-lines brackets links footnotes table drawers blocks line latex scripts timestamps tags prose; do
  for n in 1000 10000 100000 1000000; do
    out=$(timeout 60 "$bin" hostile "$parser" "$kind" "$n" "$stack" 2>&1 | grep -v '^\[')
    code=$?
    case "$out" in
      "$parser"*) echo "$out" ;;
      *) echo "$parser	$kind	$n	died: $(echo "$out" | tail -1 | cut -c1-80)" ;;
    esac
  done
done
