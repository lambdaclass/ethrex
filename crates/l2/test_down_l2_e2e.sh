#!/usr/bin/env bash
# End-to-end proof that the down-l2 pipeline delivers a signal to the right PIDs.
# Stands up two fake "ethrex" processes (node + prover), runs the real recipe's
# pipeline against them, and checks exactly which one died.
#
# Self-matching guard: `pgrep -f "ethrex l2"` also matches this script and the shell
# that runs it, because their command lines contain that string. We therefore run the
# pipeline against fake processes whose argv uses a distinct binary name, substituting
# only the `pgrep` pattern so the rest of the pipeline stays byte-identical to the
# real recipe.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

# Extract the real recipe so we test what ships, not a copy.
recipe=$(sed -n '/^down-l2:/,/^$/p' Makefile | sed -n '2,$p' | sed 's/^[[:space:]]*//' | sed '/^$/d')

# Fake "ethrex" binary: symlink tail so the process argv is `<name> -f <file>`.
FAKE=/tmp/ethrexfakedownl2
ln -sf "$(command -v tail)" "$FAKE"
: > /tmp/fk_node.log
: > /tmp/fk_prover.log
"$FAKE" -f /tmp/fk_node.log   > /dev/null & NODE=$!
"$FAKE" -f /tmp/fk_prover.log > /dev/null & PROV=$!
sleep 1

# The recipe pattern is "ethrex l2". Rewrite ONLY that pattern to match our fake
# binary's name, and resolve make's `$$` escaping the way make would, so the pipeline
# handed to the shell is byte-identical to what `make -n` shows.
fake_recipe=${recipe//ethrex l2/ethrexfakedownl2}
fake_recipe=${fake_recipe//\$\$/\$}

alive() { kill -0 "$1" 2>/dev/null && echo yes || echo no; }

echo "node PID=$NODE  prover PID=$PROV"
echo "both alive before: node=$(alive $NODE) prover=$(alive $PROV)"
echo
echo "pipeline: $fake_recipe"
echo

# Run it exactly as make would: the $$ has already been expanded to $1 by the time
# the shell sees it, so substitute that before handing it to sh.
sh -c "$fake_recipe" 2>/dev/null
sleep 1

node_after=$(alive $NODE)
prover_after=$(alive $PROV)
echo "after down-l2:  node=$node_after  prover=$prover_after"
echo

rc=0
if [ "$node_after" = "no" ]; then
    echo "ok   the L2 node received the signal"
else
    echo "FAIL the L2 node is still running -- no signal was delivered"
    rc=1
fi
if [ "$prover_after" = "yes" ]; then
    echo "ok   the prover was left running"
else
    echo "FAIL the prover was killed -- it should be excluded"
    rc=1
fi

kill $NODE $PROV 2>/dev/null
rm -f "$FAKE" /tmp/fk_node.log /tmp/fk_prover.log
exit $rc
