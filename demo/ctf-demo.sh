#!/usr/bin/env bash
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$DEMO_DIR")"
NET_NAME="stalinium-ctf"
IMAGE_NAME="stalinium-ctf:latest"
SUBNET="10.13.37.0/24"
SERVER_IP="10.13.37.2"
NUM_CLIENTS="${1:-3}"

red()   { printf '\033[1;31m%s\033[0m\n' "$*"; }
green() { printf '\033[1;32m%s\033[0m\n' "$*"; }
cyan()  { printf '\033[1;36m%s\033[0m\n' "$*"; }

cleanup() {
    cyan "[*] Cleaning up..."
    for c in $(docker ps -aq --filter "network=$NET_NAME" 2>/dev/null); do
        docker rm -f "$c" >/dev/null 2>&1 || true
    done
    docker network rm "$NET_NAME" >/dev/null 2>&1 || true
}

trap cleanup EXIT

cyan "============================================"
cyan "  Stalinium-C2 :: CTF Demo Environment"
cyan "============================================"
echo ""

cyan "[1/5] Building binaries..."
(cd "$PROJECT_ROOT/server" && cargo build --release 2>&1 | tail -1)
(cd "$PROJECT_ROOT/client" && cargo build --release 2>&1 | tail -1)

cyan "[2/5] Building container image..."
TMPDIR=$(mktemp -d)
cp "$PROJECT_ROOT/server/target/release/stalinium_server" "$TMPDIR/"
cp "$PROJECT_ROOT/client/target/release/stalinium_client" "$TMPDIR/"
cat > "$TMPDIR/Dockerfile" <<'DOCKERFILE'
FROM ubuntu:24.04
RUN mkdir -p /opt/stalinium /etc/stalinium /tmp
COPY stalinium_server stalinium_client /opt/stalinium/
RUN chmod +x /opt/stalinium/stalinium_server /opt/stalinium/stalinium_client
WORKDIR /opt/stalinium
DOCKERFILE
docker build -t "$IMAGE_NAME" "$TMPDIR" >/dev/null 2>&1
rm -rf "$TMPDIR"
green "    Image built: $IMAGE_NAME"

cyan "[3/5] Creating isolated network ($SUBNET)..."
cleanup 2>/dev/null || true
docker network create --driver bridge --subnet "$SUBNET" "$NET_NAME" >/dev/null
green "    Network: $NET_NAME"

cyan "[4/5] Launching C2 server at $SERVER_IP..."
cat > /tmp/stalinium_demo_server.toml <<EOF
[settings]
tcp_address = "0.0.0.0:4444"
admin_address = "0.0.0.0:8080"
encryption_key = "abcdefghijklmnopqrstuvwxyz123456"

[backup_servers]
primary = "${SERVER_IP}:4444"
secondary = "${SERVER_IP}:4444"
EOF

docker run -d \
    --name c2-server \
    --network "$NET_NAME" \
    --ip "$SERVER_IP" \
    -v /tmp/stalinium_demo_server.toml:/opt/stalinium/config.toml:ro \
    "$IMAGE_NAME" \
    /opt/stalinium/stalinium_server >/dev/null

sleep 2
if docker logs c2-server 2>&1 | grep -q "Listening for bots"; then
    green "    Server running (TCP :4444, Admin :8080)"
else
    red "    Server failed to start!"
    docker logs c2-server 2>&1
    exit 1
fi

cyan "[5/5] Deploying $NUM_CLIENTS implants..."
cat > /tmp/stalinium_demo_client.toml <<EOF
[settings]
server_address = "${SERVER_IP}:4444"
encryption_key = "abcdefghijklmnopqrstuvwxyz123456"

[database]
path = "/tmp/stalinium.db"
EOF

for i in $(seq 1 "$NUM_CLIENTS"); do
    CLIENT_IP="10.13.37.$((9 + i))"
    docker run -d \
        --name "implant-$i" \
        --network "$NET_NAME" \
        --ip "$CLIENT_IP" \
        -v /tmp/stalinium_demo_client.toml:/opt/stalinium/config.toml:ro \
        "$IMAGE_NAME" \
        /opt/stalinium/stalinium_client >/dev/null
done

sleep 4
CONNECTED=0
for i in $(seq 1 "$NUM_CLIENTS"); do
    if docker logs "implant-$i" 2>&1 | grep -q "PQ session established"; then
        CONNECTED=$((CONNECTED + 1))
    fi
done

if [ "$CONNECTED" -eq "$NUM_CLIENTS" ]; then
    green "    $CONNECTED/$NUM_CLIENTS implants connected (PQ handshake OK)"
else
    red "    $CONNECTED/$NUM_CLIENTS implants connected"
    for i in $(seq 1 "$NUM_CLIENTS"); do
        echo "  implant-$i:"
        docker logs "implant-$i" 2>&1 | head -5
    done
fi

echo ""
green "============================================"
green "  CTF Environment Ready"
green "============================================"
echo ""
echo "  Admin Panel: http://$SERVER_IP:8080"
echo "  API:         http://$SERVER_IP:8080/api/bots"
echo ""
echo "  Commands:"
echo "    List bots:     curl -s http://$SERVER_IP:8080/api/bots | python3 -m json.tool"
echo "    Send command:  curl -s -X POST http://$SERVER_IP:8080/api/command \\"
echo "                     -H 'Content-Type: application/json' \\"
echo "                     -d '{\"target\": \"<BOT_ADDR>\", \"command\": \"evolution_status\", \"args\": []}'"
echo "    Get results:   curl -s http://$SERVER_IP:8080/api/results | python3 -m json.tool"
echo ""
echo "  Available commands: evolution_status, compute_profile, compute_status"
echo ""
echo "  Press Ctrl+C to tear down the environment."
echo ""

while true; do sleep 3600; done
