#!/bin/bash

if [ "$#" -ne 3 ]; then
    echo "Usage: $0 <function_name> <file1.sol> <file2.sol>"
    exit 1
fi

FUNC_NAME=$1
FILE1=$2
FILE2=$3

extract_function() {
    awk -v funcname="$FUNC_NAME" '
    BEGIN {
        inside = 0
        braces = 0
        found = 0
    }

    /^[[:space:]]*function[[:space:]]+/ {
        if ($0 ~ "function[[:space:]]*" funcname "[[:space:]]*\\(") {
            inside = 1
            found = 1
        }
    }

    {
        if (inside) {
            print
            n = gsub(/{/, "{")
            braces += n
            n = gsub(/}/, "}")
            braces -= n
            if (braces == 0 && n > 0) {
                inside = 0
            }
        }
    }

    END {
        if (!found) {
            print "// ⚠️ Function not found: " funcname > "/dev/stderr"
            exit 1
        }
    }
    ' "$1"
}

TMP1=$(mktemp)
TMP2=$(mktemp)

extract_function "$FILE1" > "$TMP1"
extract_function "$FILE2" > "$TMP2"

echo "🔍 Comparing function '$FUNC_NAME' between:"
echo "    $FILE1"
echo "    $FILE2"
echo

diff --color=always "$TMP1" "$TMP2"
RESULT=$?

rm "$TMP1" "$TMP2"

if [ $RESULT -eq 0 ]; then
    echo "✅ No differences in function '$FUNC_NAME'"
else
    echo "❌ Differences found in function '$FUNC_NAME'"
    exit 1
fi
