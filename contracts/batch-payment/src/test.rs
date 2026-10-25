#![cfg(test)]
extern crate std;

use super::{BatchPayment, BatchPaymentClient, Payment};
use soroban_sdk::{
    testutils::{Address as _, AuthorizedFunction, MockAuth, MockAuthInvoke},
    token, vec, Address, Env, IntoVal, Symbol, Vec,
};

const INITIAL_BALANCE: i128 = 1_000_0000000;

struct Setup<'a> {
    env: Env,
    contract: BatchPaymentClient<'a>,
    token: token::TokenClient<'a>,
    payer: Address,
    payments: Vec<Payment>,
}

/// Registers a Stellar Asset Contract, funds the payer with an explicit
/// (non-global) admin auth for `mint`, and prepares three payments.
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    let admin = Address::generate(&env);
    let payer = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_address = sac.address();
    token::StellarAssetClient::new(&env, &token_address)
        .mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &token_address,
                fn_name: "mint",
                args: (&payer, INITIAL_BALANCE).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .mint(&payer, &INITIAL_BALANCE);

    let payments = vec![
        &env,
        Payment { to: Address::generate(&env), amount: 10_0000000 },
        Payment { to: Address::generate(&env), amount: 25_0000000 },
        Payment { to: Address::generate(&env), amount: 7_5000000 },
    ];

    let contract_id = env.register(BatchPayment, ());
    Setup {
        contract: BatchPaymentClient::new(&env, &contract_id),
        token: token::TokenClient::new(&env, &token_address),
        env,
        payer,
        payments,
    }
}

#[test]
fn batch_transfer_pays_every_recipient_when_payer_authorizes() {
    let s = setup();
    s.env.mock_all_auths();

    s.contract.batch_transfer(&s.token.address, &s.payer, &s.payments);

    // The payer (and only the payer) was asked to authorize `batch_transfer`.
    let auths = s.env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, s.payer);
    match &auths[0].1.function {
        AuthorizedFunction::Contract((contract, name, _)) => {
            assert_eq!(contract, &s.contract.address);
            assert_eq!(name, &Symbol::new(&s.env, "batch_transfer"));
        }
        other => panic!("unexpected authorized function: {other:?}"),
    }

    let mut total = 0;
    for p in s.payments.iter() {
        assert_eq!(s.token.balance(&p.to), p.amount);
        total += p.amount;
    }
    assert_eq!(s.token.balance(&s.payer), INITIAL_BALANCE - total);
}

#[test]
fn batch_transfer_is_rejected_without_payer_authorization() {
    let s = setup();

    let result = s.contract.try_batch_transfer(&s.token.address, &s.payer, &s.payments);

    assert!(result.is_err());
    assert_eq!(s.token.balance(&s.payer), INITIAL_BALANCE);
    for p in s.payments.iter() {
        assert_eq!(s.token.balance(&p.to), 0);
    }
}
