#!/usr/bin/env bash
# Install Rust if needed, clone or update pandemic-cli, then build and run it.
# Idempotent: safe to re-run; pulls latest each time.

set -euo pipefail

REPO_URL="https://github.com/emernic/pandemic-cli"
INSTALL_DIR="${PANDEMIC_CLI_DIR:-$HOME/.pandemic-cli}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust not found. Installing via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi

# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

if [ -d "$INSTALL_DIR/.git" ]; then
  echo "Updating $INSTALL_DIR..."
  git -C "$INSTALL_DIR" pull --ff-only
else
  echo "Cloning $REPO_URL into $INSTALL_DIR..."
  git clone "$REPO_URL" "$INSTALL_DIR"
fi

cd "$INSTALL_DIR"

# Build first (so build output stays on the piped stdout when run via curl|bash),
# then exec the binary with stdin reattached to the controlling TTY — needed for
# the TUI's keyboard input when this script is invoked through `curl … | bash`.
cargo build --release
if [ -e /dev/tty ]; then
  exec cargo run --release --quiet < /dev/tty
else
  exec cargo run --release --quiet
fi
