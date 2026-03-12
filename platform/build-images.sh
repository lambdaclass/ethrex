#!/bin/bash
# Build pre-built Docker images for Tokamak Platform deployments.
#
# Usage:
#   ./build-images.sh          # Build L1 + L2 images locally
#   ./build-images.sh --push   # Build and push to registry (multi-platform)
#
# Images built:
#   tokamak-appchain:l1    — L1 node
#   tokamak-appchain:l2    — L2 node + deployer + prover
#   tokamak-appchain:sp1   — L2 node with SP1 prover

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ETHREX_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REGISTRY="${ETHREX_IMAGE_REGISTRY:-}"
PLATFORMS="${ETHREX_IMAGE_PLATFORMS:-linux/amd64,linux/arm64}"

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

L1_IMAGE=$(image_name "tokamak-appchain:l1")
L2_IMAGE=$(image_name "tokamak-appchain:l2")
SP1_IMAGE=$(image_name "tokamak-appchain:sp1")

echo "==> Building platform images from $ETHREX_ROOT"
echo "    L1:  $L1_IMAGE"
echo "    L2:  $L2_IMAGE"
echo "    SP1: $SP1_IMAGE"

if $PUSH; then
  echo "    Platforms: $PLATFORMS"
fi

echo ""

build_image() {
  local target="$1"
  local image="$2"

  echo "==> Building $target image..."
  if $PUSH; then
    # Multi-platform build + push in one step (required for multi-arch manifests)
    docker buildx build \
      --platform "$PLATFORMS" \
      -f "$SCRIPT_DIR/Dockerfile" \
      --target "$target" \
      -t "$image" \
      --push \
      "$ETHREX_ROOT"
  else
    # Local build (single platform, current arch)
    docker build \
      -f "$SCRIPT_DIR/Dockerfile" \
      --target "$target" \
      -t "$image" \
      "$ETHREX_ROOT"
  fi
}

build_image l1  "$L1_IMAGE"
build_image l2  "$L2_IMAGE"
build_image sp1 "$SP1_IMAGE"

echo ""
echo "==> Images built successfully!"
echo "    $L1_IMAGE"
echo "    $L2_IMAGE"
echo "    $SP1_IMAGE"

if $PUSH; then
  echo ""
  echo "==> Multi-platform images pushed to $REGISTRY"
  echo "    Platforms: $PLATFORMS"
fi

echo ""
echo "Done. You can now deploy L2s from the platform UI."
