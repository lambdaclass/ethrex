#!/bin/bash

# This script builds tables with a benchmark comparisson for levm and revm on PR branch and on Main branch
# only for the cases where the difference on mean time is higher than 10%.

error_margin=0.1
for f in ../../../benchmark_comparison_results/*; do
            awk -F'|' -v file="${f%.md}" '      
            /`main_revm_/ {
              main_revm_row = $0;
              gsub(/±.*/, "", $3); # Remove ± part from $3 position of the row
              gsub(/[[:space:]]/, "", $3); # Remove spaces
              main_revm = $3 + 0; # Convert to number
            }
            /`main_levm_/ {
              main_levm_row = $0;
              gsub(/±.*/, "", $3);
              gsub(/[[:space:]]/, "", $3);
              main_levm = $3 + 0;
            }
            /`pr_revm_/ {
              pr_revm_row = $0;
              gsub(/±.*/, "", $3);
              gsub(/[[:space:]]/, "", $3);
              pr_revm = $3 + 0;
            }
            /`pr_levm_/ {
              pr_levm_row = $0;
              gsub(/±.*/, "", $3);
              gsub(/[[:space:]]/, "", $3);
              pr_levm = $3 + 0;
            }
            END {
              if (main_revm && main_levm && pr_revm && pr_levm) {
                revm_delta = ((main_revm > pr_revm) ? main_revm - pr_revm : pr_revm - main_revm)/(main_revm + pr_revm);
                levm_delta = ((main_levm > pr_levm) ? main_levm - pr_levm: pr_levm - main_levm)/(main_levm + pr_levm);
                if ((revm_delta > error_margin) || (levm_delta > error_margin)) {
                  printf "#### Benchmark Results: %s \n", file
                  print "| Command | Mean [s] | Min [s] | Max [s] | Relative |"
                  print "|:---|---:|---:|---:|---:|"
                  print main_revm_row
                  print main_levm_row
                  if (revm_delta > error_margin) {
                      print pr_revm_row
                  }
                  if (levm_delta > error_margin) {
                      print pr_levm_row
                  }
                }
              } else {
                printf "#### Benchmark Results: %s \n", file
                print "No results."
              }
            }
            ' "$f" >> ../../../result.md
            cat $f >> ../../../detailed_result.md
          done

if [ ! -s ../../../result.md ]; then
echo "No significant difference was registered for any benchmark run." > ../../../result.md
fi

echo "\n" >> ../../../result.md
echo "<details>\n" >> ../../../result.md
echo "<summary>Detailed view</summary>\n \n" >> ../../../result.md
echo "\`\`\`\n" >> ../../../result.md
cat ../../../detailed_result.md >> ../../../result.md
echo "\`\`\`\n" >> ../../../result.md
echo "<details>\n" >> ../../../result.md
