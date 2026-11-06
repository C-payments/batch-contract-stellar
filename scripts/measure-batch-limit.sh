#!/usr/bin/env bash
# Finds the largest batch that the deployed Multisend contract accepts, by
# binary search over the number of payments.
#
# Payments cycle through the given recipients, 1 base unit each; every
# recipient must hold a trustline for the token. The contract's own
# max_batch_size must be above the range searched (raise it temporarily
# with scripts/admin-call.sh).
#
# By default only simulations are run. Simulation does not enforce the
# per-transaction contract events size, which is the binding limit on testnet
# (a 200-payment batch simulates, a 26-payment batch fails on events size when
# applied), so SEND=1 submits each probe instead. Successful probes pay 1 base
# unit per payment plus the fee.
#
# Applied transactions used up to 125 more instructions than simulated, and
# stellar-cli 28.1.0 ignored --instruction-leeway on the protocol 30 testnet,
# so INSTRUCTIONS=<n> sets the instruction budget of every probe explicitly.
#
# Usage:   scripts/measure-batch-limit.sh <contract> <payer> <token> <max_n> <recipient>...
# Example: scripts/measure-batch-limit.sh multisend payer $TOKEN 200 r1 r2 r3 r4
#
# Network: STELLAR_NETWORK (default: testnet).
set -euo pipefail

[[ $# -ge 5 ]] || { sed -n 2,23p "$0" >&2; exit 2; }
network="${STELLAR_NETWORK:-testnet}"
send=$([[ ${SEND:-0} == 1 ]] && echo yes || echo no)
budget=()
[[ -z ${INSTRUCTIONS:-} ]] || budget=(--instructions "$INSTRUCTIONS")
contract=$1 payer=$2 token=$3 max_n=$4
shift 4

recipients=()
for r in "$@"; do
  recipients+=("$(stellar keys address "$r" 2>/dev/null || echo "$r")")
done
payer_address=$(stellar keys address "$payer" 2>/dev/null || echo "$payer")

payments_json() {
  local n=$1 i sep=""
  printf '['
  for ((i = 0; i < n; i++)); do
    printf '%s{"to":"%s","amount":"1","reference":"%064x"}' \
      "$sep" "${recipients[i % ${#recipients[@]}]}" $((i + 1))
    sep=","
  done
  printf ']'
}

# Prints nothing on success, the error line on failure.
probe() {
  stellar contract invoke --id "$contract" --source-account "$payer" --network "$network" \
    --send="$send" ${budget[@]+"${budget[@]}"} -- multisend --token "$token" --from "$payer_address" \
    --batch_id "$(openssl rand -hex 32)" --payments "$(payments_json "$1")" 2>&1 >/dev/null \
    | grep -m1 -E '^(❌|error)' || true
}

lo=1 hi=$max_n
err=$(probe "$lo")
[[ -z $err ]] || { echo "a batch of 1 fails: $err" >&2; exit 1; }
err=$(probe "$hi")
if [[ -z $err ]]; then
  echo "max_n=$hi passes; search a larger range"
  exit 0
fi
last_err=$err
while ((hi - lo > 1)); do
  mid=$(((lo + hi) / 2))
  err=$(probe "$mid")
  if [[ -z $err ]]; then lo=$mid; else hi=$mid last_err=$err; fi
  echo "n=$mid: ${err:-ok}" >&2
done
echo "largest batch that passes (send=$send): $lo"
echo "first failing size $hi: $last_err"
