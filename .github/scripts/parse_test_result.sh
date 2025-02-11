#!/bin/bash

# To determine where the test summary ends, we use new lines.

# Remove everything until the line that says "Summary: "
resulting_text=$(awk '/Summary: /, 0' $1)

empty_lines=$(echo "${resulting_text}" | awk '/^$/{print NR}')
empty_lines=($empty_lines)

resulting_text=$(echo "${resulting_text}" | sed -e "${empty_lines[0]}d")

# We substract one because we deleted one before. This correction
# shouldn't be needed if all lines are deleted as once
empty_lines[1]=$((empty_lines[1] - 1))

resulting_text=$(echo "${resulting_text}" | sed -e "${empty_lines[1]},\$d")
echo "${resulting_text}"
