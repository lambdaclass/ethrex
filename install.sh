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
cargo install --locked \
    --git $ETHREX_REPOSITORY ethrex \
    --features dev

# SETUP L1 script

curl -sSL -o ./genesis-l1-dev.json https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/test_data/genesis-l1-dev.json

cat >l1 <<EOF
#!/bin/sh
# Starts the ethrex L1

ethrex removedb
ethrex --network genesis-l1-dev.json --dev
EOF

chmod u+x ./l1
