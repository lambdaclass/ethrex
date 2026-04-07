#!/bin/sh
# Copy the pre-generated node key into the data directory, then exec ethrex.
mkdir -p /data
cp /node.key /data/node.key
exec ethrex "$@"
