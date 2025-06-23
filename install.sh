#!/bin/sh

# Fail immediately if a command exits with a non-zero status
# and treat unset variables as an error when substituting.
set -e -u

ETHREX_REPOSITORY="https://github.com/lambdaclass/ethrex.git"
OUTPUT_SUBDIR=bin

# Create output directory
mkdir -p $OUTPUT_SUBDIR
INSTALL_DIR=$(cd "$OUTPUT_SUBDIR"; pwd)

# Install ethrex
# We need to specify another branch with a fix
# TODO: remove once the fix is merged into main

# Install ethrex L1
cargo install --locked \
    --root . \
    --git $ETHREX_REPOSITORY --branch remove-nested-workspace ethrex \
    --features dev

# cargo install puts output inside `bin`
mv ./bin/ethrex $INSTALL_DIR/ethrex-l1

# Install ethrex L2
cargo install --locked \
    --root . \
    --git $ETHREX_REPOSITORY --branch remove-nested-workspace ethrex \
    --features l2,rollup_storage_libmdbx,metrics

# cargo install puts output inside `bin`
mv ./bin/ethrex $INSTALL_DIR/ethrex-l2

# SETUP L1 and L2 scripts

curl -sSL -o ./genesis-l1-dev.json https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/test_data/genesis-l1-dev.json

cat >l1 <<EOF
#!/bin/sh
# Starts the ethrex L1

$INSTALL_DIR/ethrex-l1 removedb
$INSTALL_DIR/ethrex-l1 --network genesis-l1-dev.json --dev
EOF

chmod u+x ./l1

cat >.env <<EOF
# Configuration for the ethrex L2
EOF

# TODO: move options to a .env file so user can configure them
cat >l2 <<EOF
#!/bin/sh
# Start ethrex L2 with the specified network and HTTP server settings.

# export $(cat .env | xargs)
$INSTALL_DIR/ethrex-l2 l2 init
EOF

chmod u+x ./l2
