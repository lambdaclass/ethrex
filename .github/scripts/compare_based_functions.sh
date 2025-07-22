#!/bin/bash
set -euo pipefail

FUNC_NAME="$1"
FILE1="$2"
FILE2="$3"

extract_function() {
    local FUNC="$1"
    local FILE="$2"
    awk '
    BEGIN {
        in_func = 0;
        brace_count = 0;
    }
    {
        if ($0 ~ "function[ \t]+" FUNC"\\b") {
            in_func = 1;
        }
        if (in_func) {
            print;
            n_open = gsub(/\{/, "{");
            n_close = gsub(/\}/, "}");
            brace_count += n_open - n_close;
            if (brace_count == 0 && n_open > 0) {
                exit;
            }
        }
    }' FUNC="$FUNC" "$FILE"
}

F1=$(extract_function "$FUNC_NAME" "$FILE1")
F2=$(extract_function "$FUNC_NAME" "$FILE2")

if [ "$F1" != "$F2" ]; then
    echo "❌ Functions differ!"
    diff <(echo "$F1") <(echo "$F2") || true
    exit 1
else
    echo "✅ Functions are identical."
fi
