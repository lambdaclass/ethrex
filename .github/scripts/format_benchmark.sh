#!/bin/bash

extract_benchmark() {
    local file=$1
    awk '
        /hyperfine -w 5/ { 
            inBlock = 1
            buffer = $0
            next
        }
        inBlock { 
          
            buffer = buffer "\n" $0
            if ($0 ~ /faster than/ && buffer ~ /Summary/) {
                print buffer "\n"
                inBlock = 0
            }
        }
    ' "$file"
}

INPUT_FILE=$1
OUTPUT_FILE=$2

echo '```' > "$OUTPUT_FILE"
extract_benchmark "$INPUT_FILE" >> "$OUTPUT_FILE"
echo '```' >> "$OUTPUT_FILE"
