#!/bin/sh

# Fail immediately if a command exits with a non-zero status
# and treat unset variables as an error when substituting.
set -e -u

ETHREX_REPOSITORY="https://github.com/lambdaclass/ethrex.git"

# Install ethrex
cargo install --locked \
    --git $ETHREX_REPOSITORY ethrex \
    --features dev
