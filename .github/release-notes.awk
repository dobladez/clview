# A version's section of CHANGELOG.md, as a release's notes: awk -v version=0.3.0 -f this CHANGELOG.md
# GitHub shows each line break in a release's notes, so a bullet or a paragraph goes on one line.

function flush() {
    if (line != "") print line
    line = ""
}

index($0, "## " version " ") == 1 { on = 1; next }
/^## / { flush(); on = 0 }
!on { next }
/^ *$/ || /^#/ { flush(); print; next }
/^- / { flush(); line = $0; next }
{
    sub(/^ +/, "")
    line = (line == "" ? $0 : line " " $0)
}
END { flush() }
