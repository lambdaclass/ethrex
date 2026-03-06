#!/bin/bash
# Build pre-built Docker images for Tokamak Platform deployments.
#
# Usage:
#   ./build-images.sh          # Build L1 + L2 images locally
#   ./build-images.sh --push   # Build and push to registry
#
# Images built:
#   tokamak-app-l1:latest  — L1 node
#   tokamak-app-l2:latest  — L2 node + deployer + prover

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ETHREX_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REGISTRY="${ETHREX_IMAGE_REGISTRY:-}"

PUSH=false
if [[ "${1:-}" == "--push" ]]; then
  PUSH=true
  if [[ -z "$REGISTRY" ]]; then
    echo "Error: --push requires ETHREX_IMAGE_REGISTRY env var (e.g. ghcr.io/tokamak-network)"
    exit 1
  fi
fi

image_name() {
  local name="$1"
  if [[ -n "$REGISTRY" ]]; then
    echo "$REGISTRY/$name"
  else
    echo "$name"
  fi
}

L1_IMAGE=$(image_name "tokamak-app-l1:latest")
L2_IMAGE=$(image_name "tokamak-app-l2:latest")

echo "==> Building platform images from $ETHREX_ROOT"
echo "    L1: $L1_IMAGE"
echo "    L2: $L2_IMAGE"
echo ""

echo "==> Building L1 image..."
docker build \
  -f "$SCRIPT_DIR/Dockerfile" \
  --target l1 \
  -t "$L1_IMAGE" \
  "$ETHREX_ROOT"

echo "==> Building L2 image..."
docker build \
  -f "$SCRIPT_DIR/Dockerfile" \
  --target l2 \
  -t "$L2_IMAGE" \
  "$ETHREX_ROOT"

echo ""
echo "==> Images built successfully!"
echo "    $L1_IMAGE"
echo "    $L2_IMAGE"

if $PUSH; then
  echo ""
  echo "==> Pushing images to $REGISTRY..."
  docker push "$L1_IMAGE"
  docker push "$L2_IMAGE"
  echo "==> Push complete!"
fi

echo ""
echo "Done. You can now deploy L2s from the platform UI."
