#!/bin/sh
# Install seshi on macOS or Linux (any distro, Omarchy / Arch included).
#
#   Public repo:   curl -fsSL https://raw.githubusercontent.com/CydoEntis/seshi/main/install.sh | sh
#   Private repo:  gh api -H "Accept: application/vnd.github.raw" repos/CydoEntis/seshi/contents/install.sh | sh
#
# Settings (environment variables):
#   SESHI_VERSION      a tag such as v0.1.0 (default: the latest release)
#   SESHI_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
set -eu

REPO="CydoEntis/seshi"
DIR="${SESHI_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
fail() { say "seshi install: $*" >&2; exit 1; }

case "$(uname -s)" in
  Linux) os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  *) fail "this script is for macOS and Linux; on Windows use install.ps1" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) fail "no build for $(uname -m) yet; build from source with: cargo install --git https://github.com/$REPO" ;;
esac
target="$arch-$os"
asset="seshi-$target.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# A private repo needs the GitHub CLI (signed in); a public one downloads directly.
fetch() {
  if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    if [ -n "${SESHI_VERSION:-}" ]; then
      gh release download "$SESHI_VERSION" -R "$REPO" -p "$1" -D "$tmp" --clobber
    else
      gh release download -R "$REPO" -p "$1" -D "$tmp" --clobber
    fi
  else
    if [ -n "${SESHI_VERSION:-}" ]; then
      url="https://github.com/$REPO/releases/download/$SESHI_VERSION/$1"
    else
      url="https://github.com/$REPO/releases/latest/download/$1"
    fi
    curl -fsSL "$url" -o "$tmp/$1" || fail "couldn't download $1 (a private repo needs the GitHub CLI: gh auth login)"
  fi
}

say "Downloading seshi for ${target}..."
fetch "$asset"
fetch sha256sums.txt

expected="$(grep " $asset\$" "$tmp/sha256sums.txt" | cut -d' ' -f1)"
[ -n "$expected" ] || fail "no checksum for $asset"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$asset" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$tmp/$asset" | cut -d' ' -f1)"
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch for $asset"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$DIR"
# A running seshi keeps working: the new file replaces the old one in one step.
cp "$tmp/seshi-$target/seshi" "$DIR/.seshi.new"
chmod 755 "$DIR/.seshi.new"
mv -f "$DIR/.seshi.new" "$DIR/seshi"
if [ "$(uname -s)" = Darwin ]; then
  xattr -d com.apple.quarantine "$DIR/seshi" 2>/dev/null || true
fi
say "Installed $("$DIR/seshi" --version) to $DIR/seshi"

# Another `seshi` (THC-Seshi, the password tool) found first on PATH?
found="$(command -v seshi 2>/dev/null || true)"
if [ -n "$found" ] && [ "$found" != "$DIR/seshi" ]; then
  say ""
  say "Note: '$found' comes first on your PATH (likely THC-Seshi, a different tool)."
  say "Give this one its own name, e.g. add to your shell's rc file:"
  say "  alias hy='$DIR/seshi'"
fi

case ":$PATH:" in
  *":$DIR:"*) ;;
  *)
    say ""
    say "$DIR isn't on your PATH yet. Add this to ~/.bashrc, ~/.zshrc or your shell's rc:"
    say "  export PATH=\"$DIR:\$PATH\""
    ;;
esac

say ""
say "Next: run 'seshi doctor' to check your setup, then 'seshi'."
say "If seshi was already running, restart it: seshi kill-server, then seshi."
