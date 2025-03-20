#!/bin/bash

# bench-compare.sh - Raw benchmark output wrapper
# Usage: ./bench-compare.sh file1.txt file2.txt > comparison.md

FILE1=$1
FILE2=$2

cat <<EOF
## Benchmark Raw Output Comparison

### First Implementation (ethrex-trie)
\`\`\`bash
$(while IFS= read -r line; do echo "$line"; done < "$FILE1")
\`\`\`

### Second Implementation (cita-trie)
\`\`\`bash
$(while IFS= read -r line; do echo "$line"; done < "$FILE2")
\`\`\`
EOF
