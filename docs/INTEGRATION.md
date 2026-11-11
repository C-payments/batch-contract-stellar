# Multisend integration guide

How a payout backend (C-Payroll) calls the Multisend contract, and how it reads
the results. The contract design is in [`ARCHITECTURE.md`](ARCHITECTURE.md),
section 4.2; build and deployment are in the [README](../README.md).

## Contract IDs

| Network | Multisend | USDC Stellar Asset Contract |
|---|---|---|
| Testnet | `CBRH3WONSDMADSEAO7GJHGZEKU5SZIKCJPFBPBIDLKRAKVVNJ2WQVFXK` | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` |
| Mainnet | not deployed | — |

Only pass the USDC or XLM Stellar Asset Contract as `token`. The contract
accepts any token address, so this restriction lives in the backend.

## 1. Build the batch

| Field | Rule |
|---|---|
| `from` | The company wallet that pays. It is also the transaction source account (step 3) |
| `batch_id` | 32 bytes, unique per `from`. Generate it when the batch is created and store it with the batch before asking for a signature, so a resubmission uses the same id |
| `payments[].to` | Recipient account. It must hold a trustline for the token |
| `payments[].amount` | Token base units, above zero. USDC on Stellar has 7 decimals: 1 USDC = `10_000_000` |
| `payments[].reference` | 32 opaque bytes that identify the payment record, for example a hash of the record id with a secret salt. Never put payee data on-chain |

The same recipient may appear more than once in a batch (for example salary
and bonus); each payment is transferred and reported separately.

## 2. Pre-checks before asking for a signature

1. `is_paused()` is `false`.
2. The batch has at most `max_batch_size()` payments; split a larger run into
   several batches, each with its own `batch_id`.
3. Every recipient holds a trustline for the token. A missing trustline
   reverts the whole batch.
4. The payer's token balance covers the batch total.
5. The approved draft matches the payments that will be signed.

## 3. Build, simulate and sign the transaction

- Build an `InvokeHostFunction` transaction calling
  `multisend(token, from, batch_id, payments)` with `from` as the transaction
  source account. The transaction signature then satisfies
  `from.require_auth()` and the nested token transfers, so the wallet only
  needs `stellar_signXDR` / `stellar_signAndSubmitXDR`; no
  `stellar_signAuthEntry`.
- Simulate it and apply the simulated footprint and resources.
- **Budget instructions above the simulated value.** On testnet, applied
  transactions used up to 125 more instructions than simulated and failed with
  `ResourceLimitExceeded` (`operation instructions exceeds amount specified`).
  An explicit budget about 10% above the simulated instructions worked in the
  testnet runs that used one (25 and 26 payments).
- A successful simulation does not prove the batch fits: simulation does not
  check the per-transaction events size. Rely on `max_batch_size` for that.

## 4. Submit and track

- Submit through the primary RPC; on an outage, use the fallback RPC.
- Before any retry, look the transaction up by hash. Resend the same signed
  envelope rather than building a new one: it has the same hash and sequence
  number, so it cannot be applied twice.
- If a new transaction is needed (the old one expired), keep the same
  `batch_id`. A batch that failed on-chain left its `batch_id` unused.

## 5. Errors

| Error | Code | Action |
|---|---:|---|
| `InvalidBatchSize` | 1 | Split the batch, or fix an empty one |
| `InvalidAmount` | 2 | Fix the payment amounts |
| `Overflow` | 3 | Fix the payment amounts |
| `DuplicateBatchId` | 4 | Find the original transaction and its events for `(from, batch_id)` before marking the batch executed; never resubmit under a new id without that check |
| `Paused` | 5 | Hold payouts until the admin unpauses |
| `ResourceLimitExceeded` (transaction result) | — | Raise the instruction margin, or lower `max_batch_size` |
| Token `transfer` failure | — | Check balance and trustlines; nothing was paid |

## 6. Read events and reconcile

| Event | Topics | Data (map) |
|---|---|---|
| `transfer_executed` | `"transfer_executed"`, `token`, `from`, `to` | `batch_id`, `amount`, `reference` |
| `batch_executed` | `"batch_executed"`, `token`, `from` | `batch_id`, `count`, `total` |
| `paused` / `unpaused` | `"paused"` / `"unpaused"`, `admin` | — |
| `max_batch_size_set` | `"max_batch_size_set"`, `admin` | `max_batch_size` |

- Ingest continuously with RPC `getEvents` filtered on the Multisend contract
  ID and a stored cursor; RPC keeps events only for a limited window.
- The contract is open to any caller. Keep only events whose `from` is a
  company wallet linked in C-Payroll and whose `token` is an allowed Stellar
  Asset Contract; ignore the rest.
- Match `transfer_executed` to payment records on `(from, batch_id, reference)`
  and check `batch_executed.count` and `total` against the batch. Store events
  idempotently by transaction hash and event index.
- Admin events feed the "admin action" monitor (architecture section 11).

## 7. Admin operations

The admin is a 2-of-3 multisig account (thresholds 2, master key weight 0).
Any two signers can pause, unpause or change the batch limit with
[`scripts/admin-call.sh`](../scripts/admin-call.sh):

```sh
scripts/admin-call.sh <contract> <admin> <signer> <signer> -- pause
scripts/admin-call.sh <contract> <admin> <signer> <signer> -- set_max_batch_size --max 25
```

The admin address is fixed in the contract. To replace a signer, two signers
sign a `set-options` transaction on the admin account that adds the new signer
and removes the old one (`--signer-weight 0`).

## 8. Batch limit

`max_batch_size` is 25 on testnet. Each USDC payment emits 620 bytes of
contract events and testnet allows 16,384 bytes per transaction, so the 26th
payment exceeds the limit. Measure the value again on mainnet at deployment
with [`scripts/measure-batch-limit.sh`](../scripts/measure-batch-limit.sh)
(`SEND=1`, real submissions) before setting it.
