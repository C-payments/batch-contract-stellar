#![no_std]

//! Soroban Multisend v1.
//!
//! `multisend(token, from, batch_id, payments)` pays every recipient in one
//! atomic call. The payer authorizes the call and its signature covers the
//! nested token transfers, so there is no allowance; the contract never holds
//! funds. The admin (a 2-of-3 multisig account) can pause `multisend` and
//! change the batch size limit, and nothing else. The contract is not
//! upgradable: it exposes no function that replaces its wasm.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, BytesN, Env, Vec,
};

const DAY_IN_LEDGERS: u32 = 17_280;
/// The contract instance (admin, pause flag, batch limit) and its code are
/// kept live for 30 days after each call, extended at most once a day.
const INSTANCE_TTL_EXTEND_TO: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL_EXTEND_TO - DAY_IN_LEDGERS;
/// An executed batch id is kept live for at least 30 days after the batch.
const BATCH_ID_TTL: u32 = 30 * DAY_IN_LEDGERS;

/// One recipient, the amount in token base units, and an opaque reference
/// that C-Payroll uses to match the payment to its own record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payment {
    pub to: Address,
    pub amount: i128,
    pub reference: BytesN<32>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Empty batch, batch above the limit, or a zero limit.
    InvalidBatchSize = 1,
    /// A payment amount is not above zero.
    InvalidAmount = 2,
    /// The batch total overflowed.
    Overflow = 3,
    /// This payer already executed a batch with this id.
    DuplicateBatchId = 4,
    /// `multisend` is paused by the admin.
    Paused = 5,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Paused,
    MaxBatchSize,
    /// An executed batch. Keyed by payer as well as id, so nobody can block a
    /// payer's batch by submitting their own batch with the same id first.
    Batch(Address, BytesN<32>),
}

/// Published once per payment, after its transfer.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferExecuted {
    #[topic]
    pub token: Address,
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub batch_id: BytesN<32>,
    pub amount: i128,
    pub reference: BytesN<32>,
}

/// Published once per batch, after all of its transfers.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchExecuted {
    #[topic]
    pub token: Address,
    #[topic]
    pub from: Address,
    pub batch_id: BytesN<32>,
    pub count: u32,
    pub total: i128,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paused {
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unpaused {
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaxBatchSizeSet {
    #[topic]
    pub admin: Address,
    pub max_batch_size: u32,
}

#[contract]
pub struct Multisend;

#[contractimpl]
impl Multisend {
    /// Sets the admin and the initial batch size limit. The admin is meant to
    /// be a Stellar account with three signers and thresholds of 2; it cannot
    /// be changed later, so signers are rotated on that account.
    pub fn __constructor(env: Env, admin: Address, max_batch_size: u32) {
        if max_batch_size == 0 {
            panic_with_error!(&env, Error::InvalidBatchSize);
        }
        let instance = env.storage().instance();
        instance.set(&DataKey::Admin, &admin);
        instance.set(&DataKey::Paused, &false);
        instance.set(&DataKey::MaxBatchSize, &max_batch_size);
        extend_instance_ttl(&env);
    }

    /// Transfers `token` from `from` to every recipient in `payments`.
    ///
    /// The whole batch is validated before the first transfer. If any
    /// transfer fails, the whole call is reverted and `batch_id` stays unused.
    pub fn multisend(
        env: Env,
        token: Address,
        from: Address,
        batch_id: BytesN<32>,
        payments: Vec<Payment>,
    ) -> Result<(), Error> {
        from.require_auth();
        if read_paused(&env) {
            return Err(Error::Paused);
        }

        let count = payments.len();
        if count == 0 || count > read_max_batch_size(&env) {
            return Err(Error::InvalidBatchSize);
        }
        let mut total: i128 = 0;
        for payment in payments.iter() {
            if payment.amount <= 0 {
                return Err(Error::InvalidAmount);
            }
            total = total.checked_add(payment.amount).ok_or(Error::Overflow)?;
        }

        let key = DataKey::Batch(from.clone(), batch_id.clone());
        let persistent = env.storage().persistent();
        if persistent.has(&key) {
            return Err(Error::DuplicateBatchId);
        }
        persistent.set(&key, &env.ledger().sequence());
        persistent.extend_ttl(&key, BATCH_ID_TTL, BATCH_ID_TTL);
        extend_instance_ttl(&env);

        let client = token::TokenClient::new(&env, &token);
        for payment in payments.iter() {
            client.transfer(&from, &payment.to, &payment.amount);
            TransferExecuted {
                token: token.clone(),
                from: from.clone(),
                to: payment.to,
                batch_id: batch_id.clone(),
                amount: payment.amount,
                reference: payment.reference,
            }
            .publish(&env);
        }
        BatchExecuted { token, from, batch_id, count, total }.publish(&env);
        Ok(())
    }

    /// Blocks `multisend`. Admin only.
    pub fn pause(env: Env) {
        let admin = read_admin(&env);
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &true);
        extend_instance_ttl(&env);
        Paused { admin }.publish(&env);
    }

    /// Allows `multisend` again. Admin only.
    pub fn unpause(env: Env) {
        let admin = read_admin(&env);
        admin.require_auth();
        env.storage().instance().set(&DataKey::Paused, &false);
        extend_instance_ttl(&env);
        Unpaused { admin }.publish(&env);
    }

    /// Changes the maximum number of payments in one batch. Admin only.
    pub fn set_max_batch_size(env: Env, max: u32) -> Result<(), Error> {
        let admin = read_admin(&env);
        admin.require_auth();
        if max == 0 {
            return Err(Error::InvalidBatchSize);
        }
        env.storage().instance().set(&DataKey::MaxBatchSize, &max);
        extend_instance_ttl(&env);
        MaxBatchSizeSet { admin, max_batch_size: max }.publish(&env);
        Ok(())
    }

    pub fn admin(env: Env) -> Address {
        read_admin(&env)
    }

    pub fn is_paused(env: Env) -> bool {
        read_paused(&env)
    }

    pub fn max_batch_size(env: Env) -> u32 {
        read_max_batch_size(&env)
    }
}

fn read_admin(env: &Env) -> Address {
    env.storage().instance().get(&DataKey::Admin).unwrap()
}

fn read_paused(env: &Env) -> bool {
    env.storage().instance().get(&DataKey::Paused).unwrap()
}

fn read_max_batch_size(env: &Env) -> u32 {
    env.storage().instance().get(&DataKey::MaxBatchSize).unwrap()
}

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
}

#[cfg(test)]
mod test;
