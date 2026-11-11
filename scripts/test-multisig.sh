#!/usr/bin/env bash
# Checks the 2-of-3 multisig admin of Multisend on a live network.
#
# Creates throwaway keys in a temporary stellar-cli config directory: an admin
# account, three signers and an outsider. Deploys Multisend with that admin,
# gives the admin three signers of weight 1, master key weight 0 and thresholds
# of 2, then submits admin calls with different sets of signatures through
# scripts/admin-call.sh:
#
#   one signer, the same signer twice, a signer and the outsider,
#   the outsider, the master key                       -> rejected (tx_bad_auth)
#   signers 1+2, 2+3, 1+3                              -> accepted
#
# After every call it reads the contract state and fails if the call changed
# it when it should not have, or did not when it should. Rejected transactions
# never reach the ledger, so the output of this script is their record.
#
# Usage:   scripts/test-multisig.sh [wasm]
#          (default wasm: target/wasm32v1-none/release/multisend.wasm)
# Network: STELLAR_NETWORK (default: testnet); it needs a friendbot.
set -euo pipefail

network="${STELLAR_NETWORK:-testnet}"
wasm=${1:-target/wasm32v1-none/release/multisend.wasm}
[[ -f $wasm ]] || { echo "wasm not found: $wasm" >&2; exit 2; }
admin_call="$(cd "$(dirname "$0")" && pwd)/admin-call.sh"

XDG_CONFIG_HOME=$(mktemp -d)
export XDG_CONFIG_HOME
trap 'rm -rf "$XDG_CONFIG_HOME"' EXIT

failures=0
pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*"; failures=$((failures + 1)); }

echo "Wasm: $wasm ($(shasum -a 256 "$wasm" | cut -d' ' -f1))"
echo "Network: $network"

stellar keys generate admin --network "$network" --fund -q
for k in signer-1 signer-2 signer-3 outsider; do
  stellar keys generate "$k" -q
done
admin=$(stellar keys address admin)

contract=$(stellar contract deploy --wasm "$wasm" --source-account admin --network "$network" -q \
  -- --admin "$admin" --max_batch_size 25)

for i in 1 2; do
  stellar tx new set-options --source-account admin --network "$network" -q \
    --signer "$(stellar keys address signer-$i)" --signer-weight 1 >/dev/null
done
stellar tx new set-options --source-account admin --network "$network" -q \
  --signer "$(stellar keys address signer-3)" --signer-weight 1 \
  --master-weight 0 --low-threshold 2 --med-threshold 2 --high-threshold 2 >/dev/null

echo "Contract: $contract"
echo "Admin: $admin (signers $(for i in 1 2 3; do printf '%s ' "$(stellar keys address signer-$i)"; done)master weight 0, thresholds 2)"
echo "Outsider: $(stellar keys address outsider)"
echo

state() {
  printf '%s %s' \
    "$(stellar contract invoke --id "$contract" --source-account "$admin" --network "$network" \
      --send=no -q -- is_paused)" \
    "$(stellar contract invoke --id "$contract" --source-account "$admin" --network "$network" \
      --send=no -q -- max_batch_size)"
}

# expect_rejected <description> <signer>... -- <function> [args]
expect_rejected() {
  local what=$1 before after out
  shift
  before=$(state)
  if out=$("$admin_call" "$contract" admin "$@" 2>&1); then
    fail "$what: accepted"
    return
  fi
  after=$(state)
  if ! grep -qi -E 'tx_?bad_?auth' <<<"$out"; then
    fail "$what: rejected without tx_bad_auth: $(tail -1 <<<"$out")"
  elif [[ $after != "$before" ]]; then
    fail "$what: rejected, but the state changed from '$before' to '$after'"
  else
    pass "$what: rejected (tx_bad_auth), state unchanged ($after)"
  fi
}

# expect_accepted <description> <expected state> <signer>... -- <function> [args]
expect_accepted() {
  local what=$1 want=$2 after out
  shift 2
  if ! out=$("$admin_call" "$contract" admin "$@" 2>&1); then
    fail "$what: rejected: $(tail -1 <<<"$out")"
    return
  fi
  after=$(state)
  if [[ $after == "$want" ]]; then
    pass "$what: accepted, state $after"
  else
    fail "$what: accepted, but the state is '$after', expected '$want'"
  fi
}

expect_rejected "pause, signer 1 only" signer-1 -- pause
expect_rejected "pause, signer 1 twice" signer-1 signer-1 -- pause
expect_rejected "pause, signer 1 and the outsider" signer-1 outsider -- pause
expect_rejected "pause, the outsider only" outsider -- pause
expect_rejected "pause, the master key only" admin -- pause
expect_accepted "pause, signers 1 and 2" "true 25" signer-1 signer-2 -- pause
expect_rejected "unpause, signer 3 only" signer-3 -- unpause
expect_accepted "unpause, signers 2 and 3" "false 25" signer-2 signer-3 -- unpause
expect_rejected "set_max_batch_size 10, signer 2 and the outsider" signer-2 outsider -- set_max_batch_size --max 10
expect_accepted "set_max_batch_size 10, signers 1 and 3" "false 10" signer-1 signer-3 -- set_max_batch_size --max 10

echo
if ((failures)); then
  echo "$failures check(s) failed"
  exit 1
fi
echo "All checks passed"
