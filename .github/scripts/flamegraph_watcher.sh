#!/bin/bash

# This script sends 171 * <iterations> transactions to a test account, per defined private key
# then polls the account balance until the expected balance has been reached
# and then kills the process. It also measures the elapsed time of the test and
# outputs it to Github Action's outputs.
iterations=3500
value=1
account=0x33c6b73432B3aeA0C1725E415CC40D04908B85fd
end_val=$((171 * $iterations * $value))

start_time=$(date +%s)
ethrex_l2 test load --path /home/runner/work/ethrex/ethrex/test_data/private_keys.txt -i $iterations --value $value --to $account -v &

output=$(ethrex_l2 info -b -a $account --wei 2>&1)
echo "balance: $output"
while [[ $output -lt $end_val ]]; do
    sleep 2
    output=$(ethrex_l2 info -b -a $account --wei 2>&1)
    echo "balance: $output"
done
end_time=$(date +%s)
elapsed=$((end_time - start_time))

minutes=$((elapsed / 60))
seconds=$((elapsed % 60))
output=$(ethrex_l2 info -b -a $account --wei 2>&1)
echo "Balance of $output reached in $minutes min $seconds s, killing process"

echo killing "$PROGRAM"
sudo pkill "$PROGRAM"

while pgrep -l "perf" >/dev/null; do
    echo "perf still alive, waiting for it to exit..."
    sleep 10
done
echo "perf exited"

# We need this for the following job, to add to the static page
echo "time=$minutes minutes $seconds seconds" >>"$GITHUB_OUTPUT"
