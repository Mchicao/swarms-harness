#!/usr/bin/env sh
set -eu

BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
REPO_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
BINARY_NAME="swarms-rs"
LAUNCHER="$BIN_DIR/swarm"
BINARY="$BIN_DIR/$BINARY_NAME"
MANIFEST="$REPO_DIR/rust/Cargo.toml"

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$LAUNCHER" "$BINARY"
  echo "Removed $LAUNCHER"
  echo "Removed $BINARY"
  exit 0
fi

# Build the native Rust runtime; no Python is involved anymore.
cargo build --release --manifest-path "$MANIFEST"
BUILT="$REPO_DIR/rust/target/release/$BINARY_NAME"
if [ ! -x "$BUILT" ]; then
  echo "error: release build did not produce $BUILT" >&2
  exit 1
fi

mkdir -p "$BIN_DIR"
cp -f "$BUILT" "$BINARY"
chmod +x "$BINARY"

cat > "$LAUNCHER" <<EOF
#!/usr/bin/env sh
set -eu
BINARY="$BINARY"
if [ "\$#" -eq 0 ]; then
  set -- doctor
fi
exec "\$BINARY" "\$@"
EOF
chmod +x "$LAUNCHER"
echo "Installed $LAUNCHER (native swarms-rs binary)"
echo "Add $BIN_DIR to PATH if needed."