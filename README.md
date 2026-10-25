# Soroban Batch Payment — minimal testnet prototype

> **Status: minimal testnet spike.** This is a technical proof of concept for a
> grant application, not production code. It is deployed only to Stellar
> **testnet** and has not been audited.

A single Soroban smart contract function that pays a list of recipients in one
transaction:

```rust
pub fn batch_transfer(env: Env, token: Address, from: Address, payments: Vec<Payment>)
// Payment { to: Address, amount: i128 }
```

1. `from.require_auth()` — the payer must authorize the call (standard Soroban
   auth). Nobody can use someone else's address as the payer.
2. For every `{ to, amount }` pair, `token.transfer(from, to, amount)` is called
   on the given token contract (any SEP-41 token / Stellar Asset Contract).
3. The call is atomic: if any transfer fails, the whole transaction is reverted.

The contract is in [`contracts/batch-payment/src/lib.rs`](contracts/batch-payment/src/lib.rs),
tests are in [`contracts/batch-payment/src/test.rs`](contracts/batch-payment/src/test.rs).

### Out of scope for this spike

These are planned for the next grant milestone and are intentionally **not**
implemented here: allowance-based flow, custom events with payment references,
batch size limit, contract pause, user interface. Mainnet deployment and
integration with C-Payroll payment flows are also out of scope.

## Testnet demo result

| | |
|---|---|
| Network | Stellar testnet (`Test SDF Network ; September 2015`), protocol 29 |
| Token | Circle testnet USDC `USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5` ([Circle docs](https://developers.circle.com/stablecoins/usdc-contract-addresses)) |
| Token contract (SAC) | [`CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA`](https://stellar.expert/explorer/testnet/contract/CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA) |
| **Batch contract ID** | [`CAFZNGNPQCPO2GYUWXJRNX3P3EB6H72JVJEFCXSGZTG264APPESIJNSV`](https://stellar.expert/explorer/testnet/contract/CAFZNGNPQCPO2GYUWXJRNX3P3EB6H72JVJEFCXSGZTG264APPESIJNSV) |
| Wasm hash | `1937f798371fff8badc6f88701c0cfc242f337e69ade68296a671fb51c0d32c1` |
| Wasm upload tx | [`891c9ebb135a458c35b5fb24696760bf52bd87079614d52447939edf78ab676e`](https://stellar.expert/explorer/testnet/tx/891c9ebb135a458c35b5fb24696760bf52bd87079614d52447939edf78ab676e) (ledger 5379430) |
| **Contract deploy tx** | [`0a0400024ae0e53896bcb0879c371f9b51329270e6e5fa80365dc62ad4644a70`](https://stellar.expert/explorer/testnet/tx/0a0400024ae0e53896bcb0879c371f9b51329270e6e5fa80365dc62ad4644a70) (ledger 5379431) |
| **Batch transfer tx** | [`954492f3cb3a14a0e368fe7c87a4a6b2b918d3fca0c837f6a7b7a98d1a7aa5eb`](https://stellar.expert/explorer/testnet/tx/954492f3cb3a14a0e368fe7c87a4a6b2b918d3fca0c837f6a7b7a98d1a7aa5eb) (ledger 5379435, 2026-10-25 09:32:42 UTC) |
| Payer | [`GA6PNWNR3SB3SE36PIQUCGEJJFYBEYOZRTT2U6DSCWTTFOPZBV44IOVQ`](https://stellar.expert/explorer/testnet/account/GA6PNWNR3SB3SE36PIQUCGEJJFYBEYOZRTT2U6DSCWTTFOPZBV44IOVQ) |

Raw transaction data is also available from Horizon, e.g.
`https://horizon-testnet.stellar.org/transactions/954492f3cb3a14a0e368fe7c87a4a6b2b918d3fca0c837f6a7b7a98d1a7aa5eb`.

### Batch transfer: recipients and amounts

One `batch_transfer` call, 4 recipients, 9.25 USDC in total:

| # | Recipient | Amount (USDC) | Amount (stroops) | Balance after tx |
|---|---|---:|---:|---:|
| 1 | [`GBQCZFYO4ZKMGJUE5GJGINKFDMDGOYDRKQRKFCP6BJFUB3SN7ZVR7E2P`](https://stellar.expert/explorer/testnet/account/GBQCZFYO4ZKMGJUE5GJGINKFDMDGOYDRKQRKFCP6BJFUB3SN7ZVR7E2P) | 1.0000000 | 10000000 | 1.0000000 |
| 2 | [`GDXW3WHJ2ZES66RULD2E27CVXU4HCP4L5D5OLQSICNGFHPUBWZTEAGYF`](https://stellar.expert/explorer/testnet/account/GDXW3WHJ2ZES66RULD2E27CVXU4HCP4L5D5OLQSICNGFHPUBWZTEAGYF) | 2.5000000 | 25000000 | 2.5000000 |
| 3 | [`GA5ZGO3HI5XKWT2VK7JJHL5WTJ5VNLMLHP6PK3VHPVEIXE4LBMIMIJXB`](https://stellar.expert/explorer/testnet/account/GA5ZGO3HI5XKWT2VK7JJHL5WTJ5VNLMLHP6PK3VHPVEIXE4LBMIMIJXB) | 0.7500000 | 7500000 | 0.7500000 |
| 4 | [`GCLZPK3MK6FJY2VOMYAH3C6UFGC37EJW3VXBBSSEU675AMO43B43IF3R`](https://stellar.expert/explorer/testnet/account/GCLZPK3MK6FJY2VOMYAH3C6UFGC37EJW3VXBBSSEU675AMO43B43IF3R) | 5.0000000 | 50000000 | 5.0000000 |

**Result:** transaction status `SUCCESS` (Soroban RPC `getTransaction`) /
`successful: true` (Horizon). The USDC contract emitted four `transfer` events
`payer → recipient` with exactly the amounts above. The payer's USDC balance
went from 50.0000000 to 40.7500000 (−9.25). Recipients held 0 USDC before the
call.

> Stellar testnet is periodically reset by SDF. If the links above no longer
> resolve, the demo can be reproduced with the steps below.

## Tool versions

| Tool | Version |
|---|---|
| Rust / cargo | 1.93.1 |
| Rust target | `wasm32v1-none` |
| soroban-sdk | 29.0.1 |
| stellar-cli | 28.1.0 |
| Testnet protocol | 29 |

## Build and test

```sh
rustup target add wasm32v1-none
cargo test                 # runs the two unit tests
stellar contract build     # -> target/wasm32v1-none/release/batch_payment.wasm
```

The two tests (`contracts/batch-payment/src/test.rs`) use a local Stellar Asset
Contract:

- `batch_transfer_pays_every_recipient_when_payer_authorizes` — with payer
  authorization each recipient receives its amount, the payer's balance drops by
  the total, and the only authorization requested is the payer's for
  `batch_transfer`.
- `batch_transfer_is_rejected_without_payer_authorization` — without payer
  authorization the call fails (`Error(Auth, InvalidAction)` inside the host)
  and no balances change.

## Reproduce on testnet with stellar-cli

Keys are generated locally by stellar-cli and stored in `~/.config/stellar`;
they are not part of this repository.

```sh
USDC=USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5

# 1. Accounts: a payer and 4 recipients, funded with testnet XLM by friendbot
for n in payer r1 r2 r3 r4; do stellar keys generate $n --network testnet --fund; done

# 2. USDC trustlines (a classic asset needs one before it can be received)
for n in payer r1 r2 r3 r4; do
  stellar tx new change-trust --source-account $n --line $USDC --network testnet
done

# 3. Get testnet USDC for the payer: either from https://faucet.circle.com
#    (Stellar testnet) or by buying it for XLM on the testnet DEX
#    (in the demo, 50 USDC were bought this way in tx
#    f6ef4f626359f3abce3c1dd50147c3fae96d0c6a48774d0c2764f1c81732295f)
stellar tx new path-payment-strict-receive --source-account payer \
  --send-asset native --send-max 700000000 \
  --destination payer --dest-asset $USDC --dest-amount 500000000 \
  --network testnet

# 4. Build and deploy the contract
stellar contract build
stellar contract upload --wasm target/wasm32v1-none/release/batch_payment.wasm \
  --source-account payer --network testnet          # prints the wasm hash
stellar contract deploy --wasm-hash <WASM_HASH> \
  --source-account payer --network testnet --alias batch-payment

# 5. Batch transfer (amounts are in stroops, 1 USDC = 10_000_000)
TOKEN=$(stellar contract id asset --asset $USDC --network testnet)
stellar contract invoke --id batch-payment --source-account payer --network testnet \
  -- batch_transfer \
  --token $TOKEN \
  --from $(stellar keys address payer) \
  --payments "[
    {\"to\":\"$(stellar keys address r1)\",\"amount\":\"10000000\"},
    {\"to\":\"$(stellar keys address r2)\",\"amount\":\"25000000\"},
    {\"to\":\"$(stellar keys address r3)\",\"amount\":\"7500000\"},
    {\"to\":\"$(stellar keys address r4)\",\"amount\":\"50000000\"}
  ]"

# 6. Check balances
for n in payer r1 r2 r3 r4; do
  stellar contract invoke --id $TOKEN --source-account payer --network testnet --send=no \
    -- balance --id $n
done
```

The payer signs the transaction as its source account, which also satisfies
`from.require_auth()` and the nested `transfer` authorizations of the token
contract.
