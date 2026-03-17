#!/bin/bash
# Build and push Tokamak Appchain Docker images to ghcr.io
#
# Usage:
#   ./scripts/build-docker-images.sh              # Build all, push to ghcr.io
#   ./scripts/build-docker-images.sh --no-push    # Build only, don't push
#   ./scripts/build-docker-images.sh --sp1-only   # Build SP1 image only
#
# Prerequisites:
#   - Docker installed
#   - Logged in to ghcr.io: echo $GITHUB_TOKEN | docker login ghcr.io -u USERNAME --password-stdin

set -euo pipefail

REGISTRY="ghcr.io/tokamak-network"
IMAGE="tokamak-appchain"
TAG="${IMAGE_TAG:-latest}"
PUSH=true
SP1_ONLY=false

for arg in "$@"; do
  case $arg in
    --no-push) PUSH=false ;;
    --sp1-only) SP1_ONLY=true ;;
    --tag=*) TAG="${arg#--tag=}" ;;
  esac
done

echo "=== Tokamak Appchain Docker Image Builder ==="
echo "Registry: ${REGISTRY}/${IMAGE}"
echo "Tag: ${TAG}"
echo "Push: ${PUSH}"
echo ""

# Build L1 image (standard Dockerfile, no SP1)
if [ "$SP1_ONLY" = false ]; then
  echo "[1/2] Building L1 image..."
  docker build \
    -f Dockerfile \
    -t "${REGISTRY}/${IMAGE}:l1" \
    --build-arg PROFILE=release \
    --build-arg BUILD_FLAGS="" \
    .
  echo "✅ L1 image built: ${REGISTRY}/${IMAGE}:l1"

  if [ "$PUSH" = true ]; then
    docker push "${REGISTRY}/${IMAGE}:l1"
    echo "✅ L1 image pushed"
  fi
fi

# Build SP1 image (L2 + Prover with SP1 toolchain)
echo "[2/2] Building SP1 image (this takes 20-40 minutes)..."
docker build \
  -f Dockerfile.sp1 \
  -t "${REGISTRY}/${IMAGE}:sp1" \
  -t "${REGISTRY}/${IMAGE}:${TAG}" \
  --build-arg PROFILE=release \
  --build-arg BUILD_FLAGS="--features l2,l2-sql,sp1" \
  --build-arg GUEST_PROGRAMS="evm-l2,zk-dex" \
  .
echo "✅ SP1 image built: ${REGISTRY}/${IMAGE}:sp1"

if [ "$PUSH" = true ]; then
  docker push "${REGISTRY}/${IMAGE}:sp1"
  docker push "${REGISTRY}/${IMAGE}:${TAG}"
  echo "✅ SP1 image pushed"
fi

echo ""
echo "=== Done ==="
echo "Images:"
docker images "${REGISTRY}/${IMAGE}" --format "table {{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedSince}}"
