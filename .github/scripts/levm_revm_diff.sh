#!/bin/bash


set -e
set -x

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <revm_file> <levm_file>"
    exit 1
fi

revm_file=$1
levm_file=$2

# Check if files exist
if [ ! -f "$revm_file" ]; then
    echo "Error: Revm file '$revm_file' not found"
    exit 1
fi

if [ ! -f "$levm_file" ]; then
    echo "Error: LEVM file '$levm_file' not found"
    exit 1
fi

# Create a temporary file
TEMP_FILE=$(mktemp)
trap 'rm -f $TEMP_FILE' EXIT

get_last_section() {
    tac "$1" | sed -n "1,/\*Total:/p" | tac
}

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

revm_results=$(parse_results "$revm_file")
levm_results=$(parse_results "$levm_file")

found_differences=false

echo "$revm_results" > "$TEMP_FILE"

while IFS='|' read -r name revm_passed revm_total revm_percentage; do
    if [ -n "$name" ]; then
        levm_line=$(echo "$levm_results" | grep "^$name|" || true)
        if [ -n "$levm_line" ]; then
            levm_passed=$(echo "$levm_line" | cut -d'|' -f2)
            levm_total=$(echo "$levm_line" | cut -d'|' -f3)
            levm_percentage=$(echo "$levm_line" | cut -d'|' -f4)

            if [ "$levm_passed" != "$revm_passed" ]; then
                if [ "$found_differences" = false ]; then
                    echo "Found differences between LEVM and revm: :warning:"
                    echo
                    found_differences=true
                fi
                if [ "$levm_passed" -gt "$revm_passed" ]; then
                    echo "• *$name* (improvement :arrow_up:):"
                else
                    echo "• *$name* (regression :arrow_down:):"
                fi
                echo "  - Revm: $revm_passed/$revm_total ($revm_percentage%)"
                echo "  - LEVM: $levm_passed/$levm_total ($levm_percentage%)"
                echo 1 >> "$TEMP_FILE.diff"
            fi
        else
            if [ "$found_differences" = false ]; then
                echo "Found differences between LEVM and revm: :warning:"
                echo
                found_differences=true
            fi
            echo "• *$name*: Test present in revm but missing in LEVM :x:"
            echo 1 >> "$TEMP_FILE.diff"
        fi
    fi
done < "$TEMP_FILE"

if [ ! -f "$TEMP_FILE.diff" ]; then
    echo "No differences found between revm and LEVM implementations! :white_check_mark:"
fi
