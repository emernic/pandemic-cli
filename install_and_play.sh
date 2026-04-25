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

# Build, then exec the compiled binary directly. We deliberately skip
# `cargo run` here: when this script is invoked via `curl … | bash`, cargo's
# stdio handling masks the controlling terminal from the child process and
# crossterm fails with "Failed to initialize input reader". Running the binary
# directly with stdin/stdout reattached to /dev/tty lets the TUI come up.
cargo build --release
BIN="$INSTALL_DIR/target/release/pandemic-cli"
if [ -e /dev/tty ]; then
  exec "$BIN" < /dev/tty > /dev/tty
else
  exec "$BIN"
fi
