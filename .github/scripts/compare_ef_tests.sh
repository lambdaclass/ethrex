#!/bin/bash

# $1 Main branch tests results
# $2 PR branch tests results
main_results=$(cat "$1")
IFS=$'\n' read -rd '' -a main_results <<<"${main_results}"


pr_results=$(cat "$2")
IFS=$'\n' read -rd '' -a pr_results <<<"${pr_results}"


echo "# EF Tests Comparison"
echo "|Test Name | MAIN     | PR | DIFF | "
echo "|----------|----------|----|------|"

num=0
for i in "${main_results[@]}"
do
   name_main=$(echo "$i" | awk -F " " '{print $1}')
   result_main=$(echo "$i" | awk -F " " '{print $2}')
   result_main=${result_main%(*}

   name_pr=$(echo "${pr_results[num]}" | awk -F " " '{print $1}')
   result_pr=$(echo "${pr_results[num]}" | awk -F " " '{print $2}')
   result_pr=${result_pr%(*}

   emoji=""
   if (( $(echo "$result_main > $result_pr" |bc -l) )); then
       emoji="🔴️️"
   elif (( $(echo "$result_main < $result_pr" |bc -l) )); then
       emoji="️🟢"
   else
       emoji="➖️"
   fi

   echo "|$name_main|$result_main|$result_pr| $emoji |"

   num=$((num + 1))

done
