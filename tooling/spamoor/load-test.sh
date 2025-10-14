DATADIR=/tmp/ethrex-load-test
LOG_FILE=load-test-logs.txt
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUMMARY_SCRIPT="$SCRIPT_DIR/load_test_summary.py"

rm -rf $DATADIR


echo "Compiling ethrex"
cargo b --bin ethrex --release

echo Running ethrex as a background process, saving logs to load-test-logs.txt
./target/release/ethrex --network fixtures/genesis/load-test.json --datadir $DATADIR --dev > "$LOG_FILE" 2>&1 &
ETHREX_PID=$!

echo "Ethrex running as pid ${ETHREX_PID}, waiting 2 seconds for startup..."

trap ctrl_c INT

function ctrl_c() {
        echo "** Trapped CTRL-C"
        echo "Killing background ethrex"
        kill -s INT $ETHREX_PID
}

sleep 2

echo "Starting spamoor..."

time spamoor run ./tooling/spamoor/startup.yml --rpchost="http://localhost:8545" -p 0xbcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31 -s 0

kill -s INT $ETHREX_PID

rm -rf $DATADIR

echo "Load test done"

if [[ -f "$LOG_FILE" ]]; then
        echo ""
        echo "Block execution throughput summary from $LOG_FILE:"
        if [[ -f "$SUMMARY_SCRIPT" ]]; then
                python3 "$SUMMARY_SCRIPT" "$LOG_FILE"
        else
                echo "  Summary script not found at $SUMMARY_SCRIPT; skipping throughput summary."
        fi
else
        echo "No log file $LOG_FILE found; skipping throughput summary."
fi

echo ""
