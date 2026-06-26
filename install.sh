#!/usr/bin/env sh
set -eu

BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
REPO_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
LAUNCHER="$BIN_DIR/swarm"

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$LAUNCHER"
  echo "Removed $LAUNCHER"
  exit 0
fi

mkdir -p "$BIN_DIR"
cat > "$LAUNCHER" <<EOF
#!/usr/bin/env sh
set -eu
REPO_DIR="$REPO_DIR"
if [ "\$#" -eq 0 ]; then
  set -- doctor
fi
exec python "\$REPO_DIR/scripts/swarm.py" "\$@"
EOF
chmod +x "$LAUNCHER"
echo "Installed $LAUNCHER"
echo "Add $BIN_DIR to PATH if needed."
