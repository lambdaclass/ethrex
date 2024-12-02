#!/bin/bash

output=$(cast balance 0xFCbaC0713ACf16708aB6BC977227041FA1BC618D --rpc-url=http://localhost:1729 2>&1)
end_val=$((172 * 1000 * 10000000))
echo "ini $output"
echo "end $end_val"
while [[ $output -le $end_val ]]; do
    sleep 5
    output=$(cast balance 0xFCbaC0713ACf16708aB6BC977227041FA1BC618D --rpc-url=http://localhost:1729 2>&1)
    echo "out $output"
done

echo "fin $output"
