#!/bin/sh

# Fail immediately if a command exits with a non-zero status
# and treat unset variables as an error when substituting.
set -e -u

ETHREX_REPOSITORY="https://github.com/lambdaclass/ethrex.git"

# Install ethrex
# We need to specify another branch with a fix
# TODO: Remove this once the fix is merged into main
cargo install --locked --git $ETHREX_REPOSITORY --branch remove-nested-workspace ethrex
