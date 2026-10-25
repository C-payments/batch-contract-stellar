#![no_std]

//! Minimal Soroban batch payment prototype (testnet spike).
//!
//! One entry point: `batch_transfer(token, from, payments)`. The payer must
//! authorize the call; the contract then transfers `token` from the payer to
//! each recipient in the given amount. Allowances, custom events, batch size
//! limits and pausing are intentionally out of scope.

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Vec};

/// One "recipient + amount" pair.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payment {
    pub to: Address,
    pub amount: i128,
}

#[contract]
pub struct BatchPayment;

#[contractimpl]
impl BatchPayment {
    /// Transfers `token` from `from` to every recipient in `payments`.
    ///
    /// Fails before any transfer if `from` has not authorized this invocation.
    /// If any single transfer fails, the whole call is reverted.
    pub fn batch_transfer(env: Env, token: Address, from: Address, payments: Vec<Payment>) {
        from.require_auth();

        let token = token::TokenClient::new(&env, &token);
        for payment in payments.iter() {
            token.transfer(&from, &payment.to, &payment.amount);
        }
    }
}

#[cfg(test)]
mod test;
