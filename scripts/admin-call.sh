#!/usr/bin/env bash
# Calls an admin function of Multisend from the 2-of-3 multisig admin account.
#
# The transaction is built with the admin account as its source, simulated,
# signed by each listed signer in turn and submitted. The admin account's
# thresholds are 2 and each signer has weight 1, so the network rejects the
# transaction (tx_bad_auth) unless at least two signers sign it.
#
# A signer is a stellar-cli identity, or "ledger" to sign with the connected
# Ledger device (connect each signer's device when prompted).
#
# Usage:   scripts/admin-call.sh <contract> <admin> <signer> <signer> [...] -- <function> [args]
# Example: scripts/admin-call.sh multisend ms-admin ms-signer-1 ms-signer-2 -- pause
#          scripts/admin-call.sh multisend ms-admin ms-signer-2 ms-signer-3 -- set_max_batch_size --max 20
#
# Network: STELLAR_NETWORK (default: testnet).
set -euo pipefail

usage() {
  sed -n 2,16p "$0" >&2
  exit 2
}

[[ $# -ge 4 ]] || usage
network="${STELLAR_NETWORK:-testnet}"
contract=$1
admin=$2
shift 2

signers=()
while [[ $# -gt 0 && $1 != "--" ]]; do
  signers+=("$1")
  shift
done
[[ ${1:-} == "--" && $# -ge 2 && ${#signers[@]} -ge 1 ]] || usage
shift

# The admin's master key has weight 0, so only its address is used.
admin_address=$(stellar keys address "$admin" 2>/dev/null || echo "$admin")

tx=$(stellar contract invoke --id "$contract" --source-account "$admin_address" \
  --network "$network" --build-only -- "$@")
tx=$(echo "$tx" | stellar tx simulate --source-account "$admin_address" --network "$network")

for signer in "${signers[@]}"; do
  if [[ $signer == "ledger" ]]; then
    echo "Connect the next signer's Ledger and confirm on the device." >&2
    tx=$(echo "$tx" | stellar tx sign --sign-with-ledger --network "$network")
  else
    tx=$(echo "$tx" | stellar tx sign --sign-with-key "$signer" --network "$network")
  fi
done

echo "$tx" | stellar tx send --network "$network"
