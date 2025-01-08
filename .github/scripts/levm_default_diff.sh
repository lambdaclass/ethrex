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

# Function to extract test results
parse_results() {
    grep -v "^\*" "$1" | grep ": " | while read -r line; do
        name=$(echo "$line" | cut -d':' -f1 | tr -d '\t')
        values=$(echo "$line" | cut -d':' -f2 | tr -d ' ')
        passed=$(echo "$values" | cut -d'/' -f1)
        total=$(echo "$values" | cut -d'/' -f2 | cut -d'(' -f1)
        percentage=$(echo "$values" | grep -o "[0-9.]*%" | tr -d '%')
        echo "$name|$passed|$total|$percentage"
    done
}

# Store results in temporary files
default_results=$(parse_results "$default_file")
levm_results=$(parse_results "$levm_file")

found_differences=false

echo "$default_results" | while IFS='|' read -r name default_passed default_total default_percentage; do
    if [ -n "$name" ]; then
        levm_line=$(echo "$levm_results" | grep "^$name|" || true)
        if [ -n "$levm_line" ]; then
            levm_passed=$(echo "$levm_line" | cut -d'|' -f2)
            levm_total=$(echo "$levm_line" | cut -d'|' -f3)
            levm_percentage=$(echo "$levm_line" | cut -d'|' -f4)

            if [ "$levm_passed" -lt "$default_passed" ]; then
                if [ "$found_differences" = false ]; then
                    echo "Found the following test regressions in LEVM vs default: :warning:"
                    echo
                    found_differences=true
                fi
                echo "• *$name*:"
                echo "  - Default: $default_passed/$default_total ($default_percentage%)"
                echo "  - LEVM: $levm_passed/$levm_total ($levm_percentage%)"
            fi
        else
            if [ "$found_differences" = false ]; then
                echo "Found the following test regressions in LEVM vs default: :warning:"
                echo
                found_differences=true
            fi
            echo "• *$name*: Test present in default but missing in LEVM"
        fi
    fi
done || true

if [ "$found_differences" = false ]; then
    echo "No regressions found between default and LEVM implementations! :white_check_mark:"
fi
