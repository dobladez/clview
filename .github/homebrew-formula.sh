#!/bin/sh
# Writes the Homebrew formula of a release to stdout: homebrew-formula.sh <version> <directory>,
# the directory holding the release's .sha256 files. The release workflow puts it in the tap,
# dobladez/homebrew-tap, as Formula/clv.rb beside any other formula there. ADR-0020 says why a
# tap, and why binaries.
set -eu

version=$1
directory=$2

# The checksum of one target's archive, or a stop: a formula must never go out with a wrong one.
sum() {
    file="$directory/clv-$version-$1.tar.gz.sha256"
    hash=$(cut -d ' ' -f 1 "$file")
    case "$hash" in
        *[!0-9a-f]* | "") echo "no checksum in $file" >&2; exit 1 ;;
    esac
    if [ "${#hash}" -ne 64 ]; then echo "no checksum in $file" >&2; exit 1; fi
    echo "$hash"
}

# Each once here, so that a missing one stops the script: inside the text below, it wouldn't.
mac_arm=$(sum aarch64-apple-darwin)
mac_intel=$(sum x86_64-apple-darwin)
linux_arm=$(sum aarch64-unknown-linux-musl)
linux_intel=$(sum x86_64-unknown-linux-musl)
base="https://github.com/dobladez/clview/releases/download/v$version/clv-$version"

cat <<EOF
# Written by clview's release workflow (.github/homebrew-formula.sh): an edit here goes with the next release.
class Clv < Formula
  desc "View PDFs, images, documents, spreadsheets and archives in the terminal"
  homepage "https://github.com/dobladez/clview"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "$base-aarch64-apple-darwin.tar.gz"
      sha256 "$mac_arm"
    end
    on_intel do
      url "$base-x86_64-apple-darwin.tar.gz"
      sha256 "$mac_intel"
    end
  end

  on_linux do
    on_arm do
      url "$base-aarch64-unknown-linux-musl.tar.gz"
      sha256 "$linux_arm"
    end
    on_intel do
      url "$base-x86_64-unknown-linux-musl.tar.gz"
      sha256 "$linux_intel"
    end
  end

  def install
    bin.install "clv"
    doc.install "README.md", "docs/usage.md"
  end

  test do
    assert_match "clv #{version}", shell_output("#{bin}/clv --version")
  end
end
EOF
