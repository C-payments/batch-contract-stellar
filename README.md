# Soroban Multisend

> **Status: Multisend v1 on Stellar testnet.** Not audited; the audit is
> requested through the Soroban Audit Bank. Not deployed to mainnet.

One Soroban contract call pays a list of recipients in one atomic transaction.
It is the on-chain primitive for C-Payroll payouts on Stellar; the full design
is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (section 4), and the
integration guide is [`docs/INTEGRATION.md`](docs/INTEGRATION.md).

The earlier minimal spike (`batch_transfer`, CP-968) is tagged
[`v0.1.0-spike`](https://github.com/C-payments/batch-contract-stellar/tree/v0.1.0-spike); its testnet results are in
Appendix A of the architecture document.

## Interface

```rust
pub struct Payment { pub to: Address, pub amount: i128, pub reference: BytesN<32> }

pub fn __constructor(env: Env, admin: Address, max_batch_size: u32)
pub fn multisend(env: Env, token: Address, from: Address, batch_id: BytesN<32>, payments: Vec<Payment>)
pub fn pause(env: Env)                         // admin
pub fn unpause(env: Env)                       // admin
pub fn set_max_batch_size(env: Env, max: u32)  // admin
pub fn admin(env: Env) -> Address              // read-only
pub fn is_paused(env: Env) -> bool             // read-only
pub fn max_batch_size(env: Env) -> u32         // read-only
```

`multisend`:

1. `from.require_auth()`. The payer's signature covers the nested token
   transfers, so there is no `approve` step and no standing allowance.
2. Rejects the call if the contract is paused.
3. Validates the whole batch before the first transfer: not empty, not above
   `max_batch_size`, every `amount > 0`, the total does not overflow.
4. Records `(from, batch_id)` in persistent storage and rejects a repeat.
   Keying by payer means nobody can block a payer's batch id.
5. Calls `token.transfer(from, to, amount)` for each payment and publishes
   `transfer_executed`, then `batch_executed`.

Any failure reverts the whole call: nobody is paid and `batch_id` stays unused.
The contract never holds funds and charges no fee. It is not upgradable.

| Error | Code | Meaning |
|---|---:|---|
| `InvalidBatchSize` | 1 | Empty batch, batch above the limit, or a zero limit |
| `InvalidAmount` | 2 | Amount not above zero |
| `Overflow` | 3 | Batch total overflowed |
| `DuplicateBatchId` | 4 | This payer already executed this batch id |
| `Paused` | 5 | `multisend` is paused |

| Event | Topics | Data |
|---|---|---|
| `transfer_executed` | `token`, `from`, `to` | `batch_id`, `amount`, `reference` |
| `batch_executed` | `token`, `from` | `batch_id`, `count`, `total` |
| `paused`, `unpaused` | `admin` | — |
| `max_batch_size_set` | `admin` | `max_batch_size` |

## Admin: 2-of-3 multisig

The admin is a Stellar account with three signers of weight 1, master key
weight 0 and low, medium and high thresholds of 2. Soroban checks an account's
medium threshold; the admin is the transaction source, so the transaction
signatures carry the authorization. The admin address cannot be changed, so
signers are rotated on the account; a high threshold of 2 lets two signers
replace a lost third.

[`scripts/admin-call.sh`](scripts/admin-call.sh) builds an admin call with the
admin account as source, simulates it, collects the signatures and submits it:

```sh
scripts/admin-call.sh multisend ms-admin ms-signer-1 ms-signer-2 -- pause
scripts/admin-call.sh multisend ms-admin ms-signer-2 ms-signer-3 -- unpause
scripts/admin-call.sh multisend ms-admin ms-signer-1 ms-signer-3 -- set_max_batch_size --max 25
```

A signer can be `ledger` to sign with a connected Ledger device. That path has
not been tried with a device yet.

The unit tests check that admin functions require the admin's authorization;
the 2-of-3 threshold is enforced by the network on the admin account, so it is
checked on testnet by [`scripts/test-multisig.sh`](scripts/test-multisig.sh).
The script creates throwaway keys, deploys the contract with a fresh 2-of-3
admin and submits admin calls with different sets of signatures. Calls signed
by one signer, by the same signer twice, by a signer and an outsider, by the
outsider or by the master key must fail with `tx_bad_auth` and leave the state
unchanged; calls signed by two signers must succeed. Rejected transactions never
reach the ledger, so the script's output is their record.

```sh
scripts/test-multisig.sh [wasm]   # default: target/wasm32v1-none/release/multisend.wasm
```

<details>
<summary>Run of 2026-11-11 on testnet with the CI wasm <code>67b273e2…</code>: 10 of 10 checks passed</summary>

```
Contract: CD3MNGLILPB3SOP6ZHJ2646GG26A4AFCQ2R53ZOSJGRDUT7AA642V4CI
Admin: GCJQUWNM3H7HS6WOQRHATHTLAIAFZ5KMJESO4YSGSHVZHTNDSU4VEK3H (signers GCI56D66SHV7Q6YMZUCJRLADSMPRN5KXVJR6B7NGRB7AI2JHAH7PCTJH GACK2GTYOGQK6Y5W4WZAYHN4FRX7QUXADDKCD7YE3NJ3NGKP6TLE5XB2 GAJWTPZC4X3NSBMMKTM3GORGLT7L5AYF3LMV7IT3FEL33L2X6UA3JVRW master weight 0, thresholds 2)
Outsider: GBMCF4SSTWYGLCGPGGQJR5HOLCH65S44YOEBRZYMEVBV7SNPVC3RJFB6

PASS  pause, signer 1 only: rejected (tx_bad_auth), state unchanged (false 25)
PASS  pause, signer 1 twice: rejected (tx_bad_auth), state unchanged (false 25)
PASS  pause, signer 1 and the outsider: rejected (tx_bad_auth), state unchanged (false 25)
PASS  pause, the outsider only: rejected (tx_bad_auth), state unchanged (false 25)
PASS  pause, the master key only: rejected (tx_bad_auth), state unchanged (false 25)
PASS  pause, signers 1 and 2: accepted, state true 25
PASS  unpause, signer 3 only: rejected (tx_bad_auth), state unchanged (true 25)
PASS  unpause, signers 2 and 3: accepted, state false 25
PASS  set_max_batch_size 10, signer 2 and the outsider: rejected (tx_bad_auth), state unchanged (false 25)
PASS  set_max_batch_size 10, signers 1 and 3: accepted, state false 10

All checks passed
```

</details>

## Build and test

```sh
cargo test               # 28 unit tests
stellar contract build   # -> target/wasm32v1-none/release/multisend.wasm
cargo llvm-cov --ignore-filename-regex 'test\.rs' --fail-under-lines 90   # coverage gate, as in CI
```

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml).
CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the tests with
the coverage gate, builds the contract and prints the wasm hash on every push;
a `v*` tag publishes a release with the wasm and its hash
([`release.yml`](.github/workflows/release.yml)). The same source still
builds to different bytes on different hosts: CI (ubuntu-24.04, x86_64) gives
`67b273e2…`, a macOS arm64 build gives `e8135240…`. The testnet contract is
the CI build, so check it against the CI artifact or a release, not against a
local build. Line coverage of
`src/lib.rs` is 98.1%; the uncovered lines are code generated by
`#[contracttype]`.

The tests ([`contracts/multisend/src/test.rs`](contracts/multisend/src/test.rs))
cover payouts and balances, events, payer and admin authorization, every
validation error, replay protection (including the same id from another payer),
atomicity (a failed transfer pays nobody and leaves the batch id unused), TTL
extension, pause, and changing the batch limit.

## Testnet deployment

Network: Stellar testnet, protocol 30. Token: Circle testnet USDC
`USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5`, Stellar Asset
Contract [`CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA`](https://stellar.expert/explorer/testnet/contract/CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA).

| | |
|---|---|
| **Multisend contract** | [`CBRH3WONSDMADSEAO7GJHGZEKU5SZIKCJPFBPBIDLKRAKVVNJ2WQVFXK`](https://stellar.expert/explorer/testnet/contract/CBRH3WONSDMADSEAO7GJHGZEKU5SZIKCJPFBPBIDLKRAKVVNJ2WQVFXK) |
| Wasm hash | `67b273e2816d27552a713f55300a02721088a7a7c394c7e62a5aed35a7e5bc21`, the `multisend-wasm` artifact of [CI run 42222095692](https://github.com/C-payments/batch-contract-stellar/actions/runs/42222095692) (commit `552b14c`; every CI run since `5fbc21e` prints this hash) |
| Admin account | [`GD7CQVI3M6CZEXJPXGVWGMZJ6EP2R5IZ5DNFCXAAD7RQBY2EHTMNAQHB`](https://stellar.expert/explorer/testnet/account/GD7CQVI3M6CZEXJPXGVWGMZJ6EP2R5IZ5DNFCXAAD7RQBY2EHTMNAQHB): thresholds 2/2/2, master weight 0 |
| Admin signers | `GBOGKFAH2WJZQPQAV22BHP2FWMDIXFPFFPKPSRWAPZDJVR5UB3UL4NJC`, `GCNS3E6N5U43F7FSTMMJ2FWHNC4AGGLGMBOZLG2O26PI3HYS4FPCLB45`, `GDWYXFTXMTT3BD7YAJ76EFXHEE23RPORDVYW63HKQ2XJYLECFRORSCZ6` (weight 1 each) |
| Payer | [`GA6PNWNR3SB3SE36PIQUCGEJJFYBEYOZRTT2U6DSCWTTFOPZBV44IOVQ`](https://stellar.expert/explorer/testnet/account/GA6PNWNR3SB3SE36PIQUCGEJJFYBEYOZRTT2U6DSCWTTFOPZBV44IOVQ) |
| `max_batch_size` | 25 |

The contract replaces `CA4ZE5DVT7MBG3P6TOSK3SJJNJAKFMGW2A5XSQCSOEM5XBIT5ZTTL3AD`,
deployed on 2026-11-06 from a macOS build of the same source (wasm
`e8135240…`), so that the testnet wasm is the one CI builds. The earlier log is
in this README at commit `552b14c`.

| Step | Result | Transaction |
|---|---|---|
| Add signer 1, signer 2 | ✅ | [`b9bf08f9…`](https://stellar.expert/explorer/testnet/tx/b9bf08f9095a06131ac5c93b34c2636a13742989d6ee29d0c750b9f699700f17), [`92a3ebaf…`](https://stellar.expert/explorer/testnet/tx/92a3ebaf26590b74fe763738ba2479ac391c4a97c608a9e54f8b4d36b4273f28) |
| Add signer 3, master weight 0, thresholds 2/2/2 | ✅ | [`9a9c15dd…`](https://stellar.expert/explorer/testnet/tx/9a9c15dd5248686dc005f6a70265e04aa35043cb512eb95eca0caba05a026fd9) |
| Upload the CI wasm (by the `test-multisig.sh` run) | ✅ | [`7721a3f3…`](https://stellar.expert/explorer/testnet/tx/7721a3f3f861acc695e50915ab2c29218cb10ae092ff430cc66065012f778697) |
| Deploy with admin and limit 25 | ✅ | [`60419ea1…`](https://stellar.expert/explorer/testnet/tx/60419ea19fe40002a5ff5602cc8a0b05166279420d4c39032142b14b7b91d3cb) |
| `multisend`: 4 recipients, 9.25 USDC | ✅ 4 `transfer_executed`, `batch_executed` count 4, total 92500000 | [`2abf5a39…`](https://stellar.expert/explorer/testnet/tx/2abf5a39fc5935e8cd4ca151ac782afd7bd38b34b10d260a8a162391afed1413) |
| Same `batch_id` again | ❌ `Error(Contract, #4)` `DuplicateBatchId` in simulation | — |
| `multisend` to a recipient without a USDC trustline (plus one with) | ❌ `Error(Contract, #13)` `TrustlineMissingError` from the token in simulation; nobody paid | — |
| `pause` signed by signer 1 only | ❌ `tx_bad_auth`, not included | — |
| `pause` signed by signers 1 and 2 | ✅ | [`f795a282…`](https://stellar.expert/explorer/testnet/tx/f795a28208c22632c1cd09db52311701a15e0f70c6d74d99d0c786c4105c0b5d) |
| `multisend` while paused | ❌ `Error(Contract, #5)` `Paused` in simulation | — |
| `unpause` signed by signers 2 and 3 | ✅ | [`d66d1f67…`](https://stellar.expert/explorer/testnet/tx/d66d1f67842420bdb8688b33aba259ea5accc5bffe3be672c51377e5b64d63ce) |
| `pause` signed by the master key only | ❌ `tx_bad_auth`, not included | — |
| `set_max_batch_size 26` (signers 1, 3), to check the limit | ✅ | [`043958d7…`](https://stellar.expert/explorer/testnet/tx/043958d72f30c9ce16d2b6679be32821bfcda008ee2c5d58fedd6968d67785a6) |
| `multisend`: 25 payments | ✅ | [`de93c57a…`](https://stellar.expert/explorer/testnet/tx/de93c57ac53ad9b9218dc45aca3e8334661e2fdfd8eb86732486909d11ee5690) |
| `multisend`: 26 payments | ❌ `total events size exceeds network config maximum` (16408 > 16384) | [`afcb39fc…`](https://stellar.expert/explorer/testnet/tx/afcb39fc1f44df9d333a1b1dbf222df6bfbaca40baab7c7866b154d0c78924c7) |
| `set_max_batch_size 25` (signers 1, 2) | ✅ | [`085c005b…`](https://stellar.expert/explorer/testnet/tx/085c005b1398add804df762f13dd96831f7fb8affcf89dedb3febfcb88786417) |

### Batch limit

Each USDC payment emits 620 bytes of events (248 for the token's `transfer`,
372 for `transfer_executed`), and `batch_executed` adds about 290 bytes.
Testnet allows 16,384 bytes of contract events per transaction, so 25 payments
fit and 26 do not. Simulation does not check this limit: a 200-payment batch
simulates successfully. The limit was therefore measured with real submissions
([`scripts/measure-batch-limit.sh`](scripts/measure-batch-limit.sh) with
`SEND=1`) on the earlier deployment and checked again on this one.

Applied transactions also used up to 125 more instructions than simulated
(25- and 40-payment batches failed with `operation instructions exceeds amount
specified`), and stellar-cli 28.1.0 ignored `--instruction-leeway` on the
protocol 30 testnet. Batches near the limit were sent with an explicit
`--instructions` budget about 10% above the simulated value.

## Reproduce on testnet

Keys are generated locally by stellar-cli in `~/.config/stellar`; they are not
part of this repository. The payer and recipients need USDC trustlines and the
payer needs testnet USDC (https://faucet.circle.com or the testnet DEX); see
the [spike README](https://github.com/C-payments/batch-contract-stellar/tree/v0.1.0-spike#reproduce-on-testnet-with-stellar-cli)
for those steps.

```sh
USDC=USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5
TOKEN=$(stellar contract id asset --asset $USDC --network testnet)

# 1. Admin account: three signers of weight 1, master weight 0, thresholds 2
stellar keys generate ms-admin --network testnet --fund
for i in 1 2 3; do stellar keys generate ms-signer-$i; done
for i in 1 2; do
  stellar tx new set-options --source-account ms-admin --network testnet \
    --signer $(stellar keys address ms-signer-$i) --signer-weight 1
done
stellar tx new set-options --source-account ms-admin --network testnet \
  --signer $(stellar keys address ms-signer-3) --signer-weight 1 \
  --master-weight 0 --low-threshold 2 --med-threshold 2 --high-threshold 2

# 2. Build, upload and deploy with the admin and a batch limit. A local build can
#    differ from the CI build byte for byte; to deploy the CI wasm instead, use
#    gh run download <run id> -n multisend-wasm
stellar contract build
WASM_HASH=$(stellar contract upload --wasm target/wasm32v1-none/release/multisend.wasm \
  --source-account payer --network testnet)
stellar contract deploy --wasm-hash $WASM_HASH --source-account payer --network testnet \
  --alias multisend -- --admin $(stellar keys address ms-admin) --max_batch_size 25

# 3. Multisend (amounts in base units, 1 USDC = 10_000_000; ids are 32 bytes hex)
stellar contract invoke --id multisend --source-account payer --network testnet \
  -- multisend --token $TOKEN --from $(stellar keys address payer) \
  --batch_id $(openssl rand -hex 32) \
  --payments "[
    {\"to\":\"$(stellar keys address r1)\",\"amount\":\"10000000\",\"reference\":\"$(printf '%064x' 1)\"},
    {\"to\":\"$(stellar keys address r2)\",\"amount\":\"25000000\",\"reference\":\"$(printf '%064x' 2)\"}
  ]"

# 4. Admin calls need two signers
scripts/admin-call.sh multisend ms-admin ms-signer-1 ms-signer-2 -- pause
scripts/admin-call.sh multisend ms-admin ms-signer-2 ms-signer-3 -- unpause
```

## Tool versions

| Tool | Version |
|---|---|
| Rust | 1.93.1 (pinned in `rust-toolchain.toml`), target `wasm32v1-none` |
| soroban-sdk | 29.0.1 |
| stellar-cli | 28.1.0 (testnet runs at protocol 30; the CLI warns that it supports protocol 28) |

## License

[Apache-2.0](LICENSE).
