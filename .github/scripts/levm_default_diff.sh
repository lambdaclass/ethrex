#!/bin/bash

# Exit on error, undefined vars, and pipe failures
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <default_file> <levm_file>"
    exit 1
fi

default_file=$1
levm_file=$2

# Check if files exist
if [ ! -f "$default_file" ]; then
    echo "Error: Default file '$default_file' not found"
    exit 1
fi

if [ ! -f "$levm_file" ]; then
    echo "Error: LEVM file '$levm_file' not found"
    exit 1
fi

# Create a temporary file
TEMP_FILE=$(mktemp)
trap 'rm -f $TEMP_FILE' EXIT

# Get the last section of the file (everything after the last "Total" line)
get_last_section() {
    tac "$1" | sed -n "1,/\*Total:/p" | tac
}

# Function to extract test results
parse_results() {
    while IFS= read -r line; do
        if [[ $line =~ ^[[:space:]]*[^*] && $line =~ : ]]; then
            name=$(echo "$line" | cut -d':' -f1 | tr -d '\t' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
            values=$(echo "$line" | cut -d':' -f2 | tr -d ' ')
            passed=$(echo "$values" | cut -d'/' -f1)
            total=$(echo "$values" | cut -d'/' -f2 | cut -d'(' -f1)
            percentage=$(echo "$values" | grep -o "[0-9.]*%" | tr -d '%')
            echo "$name|$passed|$total|$percentage"
        fi
    done < <(get_last_section "$1")
}

default_results=$(parse_results "$default_file")
levm_results=$(parse_results "$levm_file")

found_differences=false

echo "$default_results" > "$TEMP_FILE"

while IFS='|' read -r name default_passed default_total default_percentage; do
    if [ -n "$name" ]; then
        levm_line=$(echo "$levm_results" | grep "^$name|" || true)
        if [ -n "$levm_line" ]; then
            levm_passed=$(echo "$levm_line" | cut -d'|' -f2)
            levm_total=$(echo "$levm_line" | cut -d'|' -f3)
            levm_percentage=$(echo "$levm_line" | cut -d'|' -f4)

            if [ "$levm_passed" != "$default_passed" ]; then
                if [ "$found_differences" = false ]; then
                    echo "Found differences between LEVM and default: :warning:"
                    echo
                    found_differences=true
                fi
                if [ "$levm_passed" -gt "$default_passed" ]; then
                    echo "• *$name* (improvement :arrow_up:):"
                else
                    echo "• *$name* (regression :arrow_down:):"
                fi
                echo "  - Default: $default_passed/$default_total ($default_percentage%)"
                echo "  - LEVM: $levm_passed/$levm_total ($levm_percentage%)"
                echo 1 >> "$TEMP_FILE.diff"
            fi
        else
            if [ "$found_differences" = false ]; then
                echo "Found differences between LEVM and default: :warning:"
                echo
                found_differences=true
            fi
            echo "• *$name*: Test present in default but missing in LEVM :x:"
            echo 1 >> "$TEMP_FILE.diff"
        fi
    fi
done < "$TEMP_FILE"

if [ ! -f "$TEMP_FILE.diff" ]; then
    echo "No differences found between default and LEVM implementations! :white_check_mark:"
fi
