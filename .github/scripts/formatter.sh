#!/bin/bash

# benches/formatter.sh

FILE1=$1
FILE2=$2

cat <<EOF
# Benchmark Comparison

## ethrex-trie
\`\`\`
$(cat "$FILE1")
\`\`\`

## cita-trie
\`\`\`
$(cat "$FILE2")
\`\`\`
EOF
