#!/data/data/com.termux/files/usr/bin/bash
set -e

echo "[*] Stalinium Server -- Termux setup"
echo ""

# Install build dependencies
echo "[1/5] Installing build dependencies..."
pkg install -y rust binutils make clang 2>/dev/null || {
    echo "[!] Package install failed. Run: pkg update && pkg upgrade"
    exit 1
}

# Install termux-api for wake lock + notifications
echo "[2/5] Installing termux-api..."
pkg install -y termux-api 2>/dev/null || echo "[!] termux-api not available (notifications disabled)"

# Create data directory
echo "[3/5] Creating data directory..."
mkdir -p "$HOME/.stalinium"

# Copy config if not present
if [ ! -f "$HOME/.stalinium/config.toml" ]; then
    echo "[4/5] Installing default config..."
    cp config.termux.toml "$HOME/.stalinium/config.toml"
    echo "    Edit: $HOME/.stalinium/config.toml"
    echo "    IMPORTANT: Change encryption_key before use"
else
    echo "[4/5] Config exists, skipping."
fi

# Build
echo "[5/5] Building server (release)..."
cargo build --release

BINARY="target/release/stalinium_server"
if [ -f "$BINARY" ]; then
    echo ""
    echo "[*] Build complete: $BINARY"
    echo ""
    echo "To run:"
    echo "  ./$BINARY"
    echo ""
    echo "To install Termux:Boot auto-start:"
    echo "  1. Install Termux:Boot from F-Droid"
    echo "  2. Open Termux:Boot once to initialize"
    echo "  3. Run the server -- it will register itself"
    echo ""
    echo "Admin panel will be at http://localhost:3000"
    echo "Bots connect to port 8080"
    echo "Agentic API on port 8443"
    echo ""
    echo "Requirements on the phone:"
    echo "  - Termux:API app (F-Droid) for wake lock + notifications"
    echo "  - Termux:Boot app (F-Droid) for auto-start on reboot"
    echo "  - Disable battery optimization for Termux in Android settings"
else
    echo "[!] Build failed."
    exit 1
fi
