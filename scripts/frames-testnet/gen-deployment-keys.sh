#!/usr/bin/env bash
# Generate the key material one Frames testnet deployment needs, and print the
# config block that goes with it.
#
#   ./gen-deployment-keys.sh > /root/frames-testnet-keys.env
#
# Produces two independent BIP-39 mnemonics:
#
#   - a validator mnemonic for `preregistered_validator_keys_mnemonic`
#   - an operator mnemonic, from which the faucet, the deposit-gater admin, a
#     deployer and ten funded accounts are derived
#
# They are separate on purpose. The validator mnemonic is handed to the genesis
# generator and lives in the kurtosis config; the operator mnemonic controls
# money and the token mint. One mnemonic for both would put the gater admin's
# key in the same file as the validator keys.
#
# Nothing here may be committed. The chain's permissioning is only as good as
# these keys: an address derived from the kurtosis default mnemonic is known to
# everyone who has ever run the package, and a default-mnemonic gater admin is
# an unrevokable public token mint, because a genesis admin is granted sticky
# and `revokeRole` refuses to revoke it.
set -euo pipefail

RICH_COUNT="${RICH_COUNT:-10}"
# faucet + gater admin + deployer, then RICH_COUNT general-purpose accounts.
OPERATOR_ACCOUNTS=$((3 + RICH_COUNT))

command -v cast >/dev/null || {
  echo "cast (foundry) is required: https://getfoundry.sh" >&2
  exit 1
}

# `cast wallet new-mnemonic` prints the phrase and the derived accounts; parse
# rather than re-deriving so the addresses always match the phrase we emit.
gen() { cast wallet new-mnemonic --accounts "$1"; }

phrase_of() { sed -n '/^Phrase:/{n;p}' <<<"$1" | tr -d '\r'; }
nth_addr()  { grep '^Address:'     <<<"$1" | sed -n "$2p" | awk '{print $2}'; }
nth_key()   { grep '^Private key:' <<<"$1" | sed -n "$2p" | awk '{print $3}'; }

VALIDATOR_OUT="$(gen 1)"
OPERATOR_OUT="$(gen "$OPERATOR_ACCOUNTS")"

VALIDATOR_MNEMONIC="$(phrase_of "$VALIDATOR_OUT")"
OPERATOR_MNEMONIC="$(phrase_of "$OPERATOR_OUT")"

echo "# Frames testnet deployment keys. SECRET. Do not commit, do not copy into"
echo "# fixtures/networks/frames-testnet.yaml beyond the addresses."
echo "# Generated for a single deployment; a re-genesis needs a fresh run."
echo
echo "VALIDATOR_MNEMONIC=\"$VALIDATOR_MNEMONIC\""
echo "OPERATOR_MNEMONIC=\"$OPERATOR_MNEMONIC\""
echo
for i in 1 2 3; do
  case $i in
    1) name=FAUCET ;;
    2) name=GATER_ADMIN ;;
    3) name=DEPLOYER ;;
  esac
  echo "${name}_ADDR=$(nth_addr "$OPERATOR_OUT" $i)"
  echo "${name}_KEY=$(nth_key "$OPERATOR_OUT" $i)"
done
echo
for n in $(seq 1 "$RICH_COUNT"); do
  idx=$((3 + n))
  printf 'RICH_%02d_ADDR=%s\n' "$n" "$(nth_addr "$OPERATOR_OUT" $idx)"
  printf 'RICH_%02d_KEY=%s\n'  "$n" "$(nth_key  "$OPERATOR_OUT" $idx)"
done

echo
echo "# ---------------------------------------------------------------------"
echo "# Paste into fixtures/networks/frames-testnet.yaml, replacing the"
echo "# REPLACE_WITH_* markers. Addresses only — the keys above stay on this"
echo "# host."
echo "# ---------------------------------------------------------------------"
echo "#"
echo "#   preregistered_validator_keys_mnemonic: \"$VALIDATOR_MNEMONIC\""
echo "#"
echo "#   prefunded_accounts: '{"
echo "#     \"$(nth_addr "$OPERATOR_OUT" 1)\": {\"balance\": \"1000000ETH\"},"
echo "#     \"$(nth_addr "$OPERATOR_OUT" 2)\": {\"balance\": \"1000ETH\"},"
echo "#     \"$(nth_addr "$OPERATOR_OUT" 3)\": {\"balance\": \"1000ETH\"},"
for n in $(seq 1 "$RICH_COUNT"); do
  idx=$((3 + n))
  sep=","
  [ "$n" -eq "$RICH_COUNT" ] && sep=""
  echo "#     \"$(nth_addr "$OPERATOR_OUT" $idx)\": {\"balance\": \"100000ETH\"}$sep"
done
echo "#   }'"
echo "#"
echo "#   DEPOSIT_CONTRACT_ADMINS: '[\"$(nth_addr "$OPERATOR_OUT" 2)\"]'"
echo "#"
echo "# The gater admin appears twice — in prefunded_accounts and in"
echo "# DEPOSIT_CONTRACT_ADMINS — and both must be the same address, or the"
echo "# account that can mint deposit tokens is not the one holding gas."
