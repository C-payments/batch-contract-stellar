#![cfg(test)]
extern crate std;

use super::{
    BatchExecuted, DataKey, Error, MaxBatchSizeSet, Multisend, MultisendArgs, MultisendClient,
    Paused, Payment, TransferExecuted, Unpaused, BATCH_ID_TTL, INSTANCE_TTL_EXTEND_TO,
};
use soroban_sdk::{
    testutils::{
        storage::{Instance as _, Persistent as _},
        Address as _, AuthorizedFunction, AuthorizedInvocation, Events as _, MockAuth,
        MockAuthInvoke,
    },
    token, Address, BytesN, Env, Event as _, IntoVal, Symbol, Val, Vec,
};

const MAX_BATCH_SIZE: u32 = 5;
const INITIAL_BALANCE: i128 = 1_000_0000000;

struct Setup<'a> {
    env: Env,
    contract: MultisendClient<'a>,
    token: token::TokenClient<'a>,
    token_issuer: token::StellarAssetClient<'a>,
    admin: Address,
    payer: Address,
}

/// Registers a Stellar Asset Contract and the Multisend contract with a
/// batch limit of `MAX_BATCH_SIZE`, and funds the payer. No authorization is
/// mocked globally: each test authorizes exactly the calls it needs.
fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    let admin = Address::generate(&env);
    let payer = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let token_issuer = token::StellarAssetClient::new(&env, &sac.address());
    token_issuer.mock_all_auths().mint(&payer, &INITIAL_BALANCE);

    let contract_id = env.register(
        Multisend,
        MultisendArgs::__constructor(&admin, &MAX_BATCH_SIZE),
    );
    Setup {
        contract: MultisendClient::new(&env, &contract_id),
        token: token::TokenClient::new(&env, &sac.address()),
        token_issuer,
        env,
        admin,
        payer,
    }
}

fn id(env: &Env, n: u8) -> BytesN<32> {
    BytesN::from_array(env, &[n; 32])
}

/// One payment per amount, each to a new recipient with its own reference.
fn payments(env: &Env, amounts: &[i128]) -> Vec<Payment> {
    let mut payments = Vec::new(env);
    for (i, amount) in amounts.iter().enumerate() {
        payments.push_back(Payment {
            to: Address::generate(env),
            amount: *amount,
            reference: id(env, i as u8 + 1),
        });
    }
    payments
}

fn total(payments: &Vec<Payment>) -> i128 {
    payments.iter().map(|p| p.amount).sum()
}

fn assert_nobody_paid(s: &Setup, payer: &Address, payer_balance: i128, payments: &Vec<Payment>) {
    assert_eq!(s.token.balance(payer), payer_balance);
    for p in payments.iter() {
        assert_eq!(s.token.balance(&p.to), 0);
    }
}

/// Authorizes only `signer` for the next call to `fn_name` with `args`.
fn authorize_only(s: &Setup, signer: &Address, fn_name: &str, args: Vec<Val>) {
    s.env.mock_auths(&[MockAuth {
        address: signer,
        invoke: &MockAuthInvoke {
            contract: &s.contract.address,
            fn_name,
            args,
            sub_invokes: &[],
        },
    }]);
}

fn root_auth(s: &Setup, signer: &Address, fn_name: &str, args: Vec<Val>) -> (Address, AuthorizedInvocation) {
    (
        signer.clone(),
        AuthorizedInvocation {
            function: AuthorizedFunction::Contract((
                s.contract.address.clone(),
                Symbol::new(&s.env, fn_name),
                args,
            )),
            sub_invocations: std::vec![],
        },
    )
}

// --- Constructor ---

#[test]
fn constructor_stores_admin_and_batch_limit() {
    let s = setup();

    assert_eq!(s.contract.admin(), s.admin);
    assert_eq!(s.contract.max_batch_size(), MAX_BATCH_SIZE);
    assert!(!s.contract.is_paused());
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn constructor_rejects_zero_batch_limit() {
    let env = Env::default();
    env.register(
        Multisend,
        MultisendArgs::__constructor(&Address::generate(&env), &0),
    );
}

// --- Successful multisend ---

#[test]
fn multisend_pays_every_recipient_and_payer_loses_total() {
    let s = setup();
    let payments = payments(&s.env, &[10_0000000, 25_0000000, 7_5000000]);

    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &id(&s.env, 0xB1), &payments);

    for p in payments.iter() {
        assert_eq!(s.token.balance(&p.to), p.amount);
    }
    assert_eq!(s.token.balance(&s.payer), INITIAL_BALANCE - total(&payments));
    assert_eq!(s.token.balance(&s.contract.address), 0);
}

#[test]
fn multisend_requires_only_payer_authorization() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);
    let payments = payments(&s.env, &[10_0000000, 25_0000000]);

    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &batch_id, &payments);

    // One root authorization, by the payer, for `multisend` with these exact
    // arguments. The nested token transfers are covered by it.
    let auths = s.env.auths();
    assert_eq!(auths.len(), 1);
    let (signer, invocation) = &auths[0];
    assert_eq!(signer, &s.payer);
    assert_eq!(
        invocation.function,
        AuthorizedFunction::Contract((
            s.contract.address.clone(),
            Symbol::new(&s.env, "multisend"),
            (&s.token.address, &s.payer, &batch_id, &payments).into_val(&s.env),
        ))
    );
    assert_eq!(invocation.sub_invocations.len(), payments.len() as usize);
}

#[test]
fn multisend_emits_transfer_event_per_payment_and_batch_event() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);
    let payments = payments(&s.env, &[10_0000000, 25_0000000]);

    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &batch_id, &payments);

    let mut expected = std::vec::Vec::new();
    for p in payments.iter() {
        expected.push(
            TransferExecuted {
                token: s.token.address.clone(),
                from: s.payer.clone(),
                to: p.to,
                batch_id: batch_id.clone(),
                amount: p.amount,
                reference: p.reference,
            }
            .to_xdr(&s.env, &s.contract.address),
        );
    }
    expected.push(
        BatchExecuted {
            token: s.token.address.clone(),
            from: s.payer.clone(),
            batch_id: batch_id.clone(),
            count: 2,
            total: total(&payments),
        }
        .to_xdr(&s.env, &s.contract.address),
    );
    assert_eq!(s.env.events().all().filter_by_contract(&s.contract.address), expected);
}

#[test]
fn same_recipient_twice_receives_both_payments() {
    let s = setup();
    let recipient = Address::generate(&s.env);
    let mut payments = Vec::new(&s.env);
    payments.push_back(Payment { to: recipient.clone(), amount: 100_0000000, reference: id(&s.env, 1) });
    payments.push_back(Payment { to: recipient.clone(), amount: 20_0000000, reference: id(&s.env, 2) });

    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &id(&s.env, 0xB1), &payments);

    assert_eq!(s.token.balance(&recipient), 120_0000000);
}

#[test]
fn multisend_accepts_batch_at_limit() {
    let s = setup();
    let payments = payments(&s.env, &[1; MAX_BATCH_SIZE as usize]);

    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &id(&s.env, 0xB1), &payments);

    assert_eq!(s.token.balance(&s.payer), INITIAL_BALANCE - MAX_BATCH_SIZE as i128);
}

// --- Authorization ---

#[test]
fn multisend_without_payer_authorization_is_rejected() {
    let s = setup();
    let payments = payments(&s.env, &[10_0000000, 25_0000000]);

    let result = s.contract.try_multisend(&s.token.address, &s.payer, &id(&s.env, 0xB1), &payments);

    assert!(result.is_err());
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);
}

#[test]
fn multisend_authorized_by_someone_else_is_rejected() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);
    let payments = payments(&s.env, &[10_0000000]);
    let thief = Address::generate(&s.env);
    authorize_only(
        &s,
        &thief,
        "multisend",
        (&s.token.address, &s.payer, &batch_id, &payments).into_val(&s.env),
    );

    let result = s.contract.try_multisend(&s.token.address, &s.payer, &batch_id, &payments);

    assert!(result.is_err());
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);
}

// --- Validation ---

#[test]
fn multisend_rejects_empty_batch() {
    let s = setup();

    let result = s.contract.mock_all_auths().try_multisend(
        &s.token.address,
        &s.payer,
        &id(&s.env, 0xB1),
        &Vec::new(&s.env),
    );

    assert_eq!(result, Err(Ok(Error::InvalidBatchSize)));
}

#[test]
fn multisend_rejects_batch_above_limit() {
    let s = setup();
    let payments = payments(&s.env, &[1; MAX_BATCH_SIZE as usize + 1]);

    let result = s.contract.mock_all_auths().try_multisend(
        &s.token.address,
        &s.payer,
        &id(&s.env, 0xB1),
        &payments,
    );

    assert_eq!(result, Err(Ok(Error::InvalidBatchSize)));
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);
}

#[test]
fn multisend_rejects_zero_amount() {
    let s = setup();
    let payments = payments(&s.env, &[10_0000000, 0]);

    let result = s.contract.mock_all_auths().try_multisend(
        &s.token.address,
        &s.payer,
        &id(&s.env, 0xB1),
        &payments,
    );

    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);
}

#[test]
fn multisend_rejects_negative_amount() {
    let s = setup();
    let payments = payments(&s.env, &[-1]);

    let result = s.contract.mock_all_auths().try_multisend(
        &s.token.address,
        &s.payer,
        &id(&s.env, 0xB1),
        &payments,
    );

    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);
}

#[test]
fn invalid_amount_late_in_batch_pays_nobody() {
    let s = setup();
    let payments = payments(&s.env, &[10_0000000, 25_0000000, 7_5000000, -5]);

    let result = s.contract.mock_all_auths().try_multisend(
        &s.token.address,
        &s.payer,
        &id(&s.env, 0xB1),
        &payments,
    );

    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);
}

#[test]
fn multisend_rejects_total_overflow() {
    let s = setup();
    let payments = payments(&s.env, &[i128::MAX, 1]);

    let result = s.contract.mock_all_auths().try_multisend(
        &s.token.address,
        &s.payer,
        &id(&s.env, 0xB1),
        &payments,
    );

    assert_eq!(result, Err(Ok(Error::Overflow)));
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);
}

// --- Replay protection ---

#[test]
fn multisend_rejects_duplicate_batch_id_from_same_payer() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);
    let first = payments(&s.env, &[10_0000000]);
    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &batch_id, &first);
    let balance_after_first = s.token.balance(&s.payer);

    let second = payments(&s.env, &[10_0000000]);
    let result = s
        .contract
        .mock_all_auths()
        .try_multisend(&s.token.address, &s.payer, &batch_id, &second);

    assert_eq!(result, Err(Ok(Error::DuplicateBatchId)));
    assert_nobody_paid(&s, &s.payer, balance_after_first, &second);
}

#[test]
fn same_batch_id_from_another_payer_is_accepted() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);
    let other_payer = Address::generate(&s.env);
    s.token_issuer.mock_all_auths().mint(&other_payer, &INITIAL_BALANCE);

    // Another payer uses the batch id first; the payer's batch must still go through.
    s.contract.mock_all_auths().multisend(
        &s.token.address,
        &other_payer,
        &batch_id,
        &payments(&s.env, &[1]),
    );
    let payments = payments(&s.env, &[10_0000000]);
    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &batch_id, &payments);

    assert_eq!(s.token.balance(&payments.get(0).unwrap().to), 10_0000000);
}

#[test]
fn failed_transfer_reverts_batch_and_leaves_batch_id_unused() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);
    let payer = Address::generate(&s.env);
    s.token_issuer.mock_all_auths().mint(&payer, &10_0000000);
    // The second transfer exceeds the remaining balance.
    let payments = payments(&s.env, &[6_0000000, 6_0000000]);

    let result = s
        .contract
        .mock_all_auths()
        .try_multisend(&s.token.address, &payer, &batch_id, &payments);

    assert!(result.is_err());
    assert_nobody_paid(&s, &payer, 10_0000000, &payments);

    // After funding, the same batch id goes through.
    s.token_issuer.mock_all_auths().mint(&payer, &2_0000000);
    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &payer, &batch_id, &payments);
    assert_eq!(s.token.balance(&payer), 0);
}

#[test]
fn multisend_extends_batch_id_and_instance_ttl() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);

    s.contract.mock_all_auths().multisend(
        &s.token.address,
        &s.payer,
        &batch_id,
        &payments(&s.env, &[1]),
    );

    let (batch_ttl, instance_ttl) = s.env.as_contract(&s.contract.address, || {
        (
            s.env
                .storage()
                .persistent()
                .get_ttl(&DataKey::Batch(s.payer.clone(), batch_id.clone())),
            s.env.storage().instance().get_ttl(),
        )
    });
    assert_eq!(batch_ttl, BATCH_ID_TTL);
    assert_eq!(instance_ttl, INSTANCE_TTL_EXTEND_TO);
}

// --- Pause ---

#[test]
fn paused_contract_rejects_multisend_until_unpaused() {
    let s = setup();
    let batch_id = id(&s.env, 0xB1);
    let payments = payments(&s.env, &[10_0000000]);

    s.contract.mock_all_auths().pause();
    assert!(s.contract.is_paused());
    let result = s
        .contract
        .mock_all_auths()
        .try_multisend(&s.token.address, &s.payer, &batch_id, &payments);
    assert_eq!(result, Err(Ok(Error::Paused)));
    assert_nobody_paid(&s, &s.payer, INITIAL_BALANCE, &payments);

    s.contract.mock_all_auths().unpause();
    assert!(!s.contract.is_paused());
    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &batch_id, &payments);
    assert_eq!(s.token.balance(&payments.get(0).unwrap().to), 10_0000000);
}

#[test]
fn pause_blocks_only_multisend() {
    let s = setup();
    s.contract.mock_all_auths().pause();

    s.contract.mock_all_auths().set_max_batch_size(&3);

    assert_eq!(s.contract.max_batch_size(), 3);
    assert_eq!(s.contract.admin(), s.admin);
}

// --- Admin ---

#[test]
fn admin_functions_are_authorized_by_admin() {
    let s = setup();

    s.contract.mock_all_auths().pause();
    assert_eq!(s.env.auths(), [root_auth(&s, &s.admin, "pause", ().into_val(&s.env))]);

    s.contract.mock_all_auths().unpause();
    assert_eq!(s.env.auths(), [root_auth(&s, &s.admin, "unpause", ().into_val(&s.env))]);

    s.contract.mock_all_auths().set_max_batch_size(&3);
    assert_eq!(
        s.env.auths(),
        [root_auth(&s, &s.admin, "set_max_batch_size", (3u32,).into_val(&s.env))]
    );
}

#[test]
fn pause_without_admin_authorization_is_rejected() {
    let s = setup();
    let stranger = Address::generate(&s.env);

    assert!(s.contract.try_pause().is_err());
    authorize_only(&s, &stranger, "pause", ().into_val(&s.env));
    assert!(s.contract.try_pause().is_err());

    assert!(!s.contract.is_paused());
}

#[test]
fn unpause_without_admin_authorization_is_rejected() {
    let s = setup();
    let stranger = Address::generate(&s.env);
    s.contract.mock_all_auths().pause();

    assert!(s.contract.try_unpause().is_err());
    authorize_only(&s, &stranger, "unpause", ().into_val(&s.env));
    assert!(s.contract.try_unpause().is_err());

    assert!(s.contract.is_paused());
}

#[test]
fn set_max_batch_size_without_admin_authorization_is_rejected() {
    let s = setup();
    let stranger = Address::generate(&s.env);

    assert!(s.contract.try_set_max_batch_size(&100).is_err());
    authorize_only(&s, &stranger, "set_max_batch_size", (100u32,).into_val(&s.env));
    assert!(s.contract.try_set_max_batch_size(&100).is_err());

    assert_eq!(s.contract.max_batch_size(), MAX_BATCH_SIZE);
}

#[test]
fn set_max_batch_size_applies_to_next_batch() {
    let s = setup();

    // Lowered: a batch that fitted the old limit is now rejected.
    s.contract.mock_all_auths().set_max_batch_size(&2);
    let three = payments(&s.env, &[1, 1, 1]);
    let result = s.contract.mock_all_auths().try_multisend(
        &s.token.address,
        &s.payer,
        &id(&s.env, 0xB1),
        &three,
    );
    assert_eq!(result, Err(Ok(Error::InvalidBatchSize)));

    // Raised: a batch above the old limit now goes through.
    s.contract.mock_all_auths().set_max_batch_size(&(MAX_BATCH_SIZE + 1));
    let six = payments(&s.env, &[1; MAX_BATCH_SIZE as usize + 1]);
    s.contract
        .mock_all_auths()
        .multisend(&s.token.address, &s.payer, &id(&s.env, 0xB2), &six);
    assert_eq!(s.token.balance(&s.payer), INITIAL_BALANCE - six.len() as i128);
}

#[test]
fn set_max_batch_size_rejects_zero() {
    let s = setup();

    let result = s.contract.mock_all_auths().try_set_max_batch_size(&0);

    assert_eq!(result, Err(Ok(Error::InvalidBatchSize)));
    assert_eq!(s.contract.max_batch_size(), MAX_BATCH_SIZE);
}

#[test]
fn admin_actions_emit_events() {
    let s = setup();
    let events = |s: &Setup| s.env.events().all().filter_by_contract(&s.contract.address);

    s.contract.mock_all_auths().pause();
    assert_eq!(events(&s), [Paused { admin: s.admin.clone() }.to_xdr(&s.env, &s.contract.address)]);

    s.contract.mock_all_auths().unpause();
    assert_eq!(events(&s), [Unpaused { admin: s.admin.clone() }.to_xdr(&s.env, &s.contract.address)]);

    s.contract.mock_all_auths().set_max_batch_size(&3);
    assert_eq!(
        events(&s),
        [MaxBatchSizeSet { admin: s.admin.clone(), max_batch_size: 3 }
            .to_xdr(&s.env, &s.contract.address)]
    );
}

