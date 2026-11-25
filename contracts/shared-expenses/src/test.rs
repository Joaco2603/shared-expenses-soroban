#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

struct Group {
    env: Env,
    contract_id: Address,
    a: Address,
    b: Address,
    c: Address,
}

impl Group {
    fn client(&self) -> SharedExpensesClient<'_> {
        SharedExpensesClient::new(&self.env, &self.contract_id)
    }

    fn assert_sums_to_zero(&self) {
        let client = self.client();
        let total: i128 = [&self.a, &self.b, &self.c]
            .iter()
            .map(|m| client.balance(*m))
            .sum();
        assert_eq!(total, 0);
    }
}

fn setup() -> Group {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SharedExpenses, ());
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);

    SharedExpensesClient::new(&env, &contract_id).init(&vec![
        &env,
        a.clone(),
        b.clone(),
        c.clone(),
    ]);

    Group {
        env,
        contract_id,
        a,
        b,
        c,
    }
}

#[test]
fn init_stores_members_and_zero_balances() {
    let g = setup();
    let client = g.client();
    assert_eq!(client.members().len(), 3);
    assert_eq!(client.balance(&g.a), 0);
    assert_eq!(client.expenses().len(), 0);
}

#[test]
fn init_twice_fails() {
    let g = setup();
    let res = g.client().try_init(&vec![&g.env, g.a.clone(), g.b.clone()]);
    assert!(res.is_err());
}

#[test]
fn init_rejects_single_member_and_duplicates() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SharedExpenses, ());
    let client = SharedExpensesClient::new(&env, &contract_id);
    let a = Address::generate(&env);

    assert!(client.try_init(&vec![&env, a.clone()]).is_err());
    assert!(client.try_init(&vec![&env, a.clone(), a.clone()]).is_err());
}

#[test]
fn equal_split_gives_remainder_to_first_participants() {
    let g = setup();
    // 100 split 3 ways: shares are 34, 33, 33.
    g.client().add_expense(
        &g.a,
        &100,
        &vec![&g.env, g.a.clone(), g.b.clone(), g.c.clone()],
    );

    let client = g.client();
    assert_eq!(client.balance(&g.a), 100 - 34);
    assert_eq!(client.balance(&g.b), -33);
    assert_eq!(client.balance(&g.c), -33);
    g.assert_sums_to_zero();
}

#[test]
fn payer_not_in_participants_is_owed_full_amount() {
    let g = setup();
    // b pays 60 for a and c only: each owes 30.
    g.client()
        .add_expense(&g.b, &60, &vec![&g.env, g.a.clone(), g.c.clone()]);

    let client = g.client();
    assert_eq!(client.balance(&g.b), 60);
    assert_eq!(client.balance(&g.a), -30);
    assert_eq!(client.balance(&g.c), -30);
    g.assert_sums_to_zero();
}

#[test]
fn expenses_are_logged() {
    let g = setup();
    g.client()
        .add_expense(&g.a, &50, &vec![&g.env, g.a.clone(), g.b.clone()]);
    let log = g.client().expenses();
    assert_eq!(log.len(), 1);
    assert_eq!(log.get(0).unwrap().amount, 50);
}

#[test]
fn rejects_invalid_expenses() {
    let g = setup();
    let client = g.client();
    let all = vec![&g.env, g.a.clone(), g.b.clone()];
    let outsider = Address::generate(&g.env);

    assert!(client.try_add_expense(&g.a, &0, &all).is_err());
    assert!(client.try_add_expense(&g.a, &-5, &all).is_err());
    assert!(client.try_add_expense(&g.a, &10, &vec![&g.env]).is_err());
    assert!(client.try_add_expense(&outsider, &10, &all).is_err());
    assert!(client
        .try_add_expense(&g.a, &10, &vec![&g.env, g.a.clone(), g.a.clone()])
        .is_err());
    assert!(client
        .try_add_expense(&g.a, &10, &vec![&g.env, g.a.clone(), outsider.clone()])
        .is_err());

    // Nothing was registered by the failed calls.
    assert_eq!(client.expenses().len(), 0);
    assert_eq!(client.balance(&g.a), 0);
}

#[test]
fn record_payment_reduces_debt() {
    let g = setup();
    g.client().add_expense(
        &g.a,
        &90,
        &vec![&g.env, g.a.clone(), g.b.clone(), g.c.clone()],
    );
    // b owes 30 to a.
    g.client().record_payment(&g.b, &g.a, &10);

    let client = g.client();
    assert_eq!(client.balance(&g.b), -20);
    assert_eq!(client.balance(&g.a), 60 - 10);
    g.assert_sums_to_zero();
}

#[test]
fn record_payment_rejects_overpayment_and_wrong_direction() {
    let g = setup();
    g.client().add_expense(
        &g.a,
        &90,
        &vec![&g.env, g.a.clone(), g.b.clone(), g.c.clone()],
    );
    let client = g.client();

    // b owes only 30.
    assert!(client.try_record_payment(&g.b, &g.a, &31).is_err());
    // a is owed, so a cannot pay b.
    assert!(client.try_record_payment(&g.a, &g.b, &5).is_err());
    // Self payments are not allowed.
    assert!(client.try_record_payment(&g.b, &g.b, &5).is_err());
    // Balances unchanged.
    assert_eq!(client.balance(&g.b), -30);
}

#[test]
fn settle_plan_clears_all_balances() {
    let g = setup();
    g.client().add_expense(
        &g.a,
        &120,
        &vec![&g.env, g.a.clone(), g.b.clone(), g.c.clone()],
    );
    g.client()
        .add_expense(&g.c, &50, &vec![&g.env, g.b.clone(), g.c.clone()]);
    g.assert_sums_to_zero();

    let client = g.client();
    let plan = client.settle_plan();
    assert!(plan.len() <= 2, "at most members - 1 transfers");

    // Apply every transfer in the plan; everyone must end at zero.
    for i in 0..plan.len() {
        let t = plan.get(i).unwrap();
        client.record_payment(&t.from, &t.to, &t.amount);
    }
    assert_eq!(client.balance(&g.a), 0);
    assert_eq!(client.balance(&g.b), 0);
    assert_eq!(client.balance(&g.c), 0);
    assert_eq!(client.settle_plan().len(), 0);
}

#[test]
fn settle_plan_is_read_only() {
    let g = setup();
    g.client().add_expense(
        &g.a,
        &30,
        &vec![&g.env, g.a.clone(), g.b.clone(), g.c.clone()],
    );
    let client = g.client();
    let _ = client.settle_plan();
    assert_eq!(client.balance(&g.a), 20);
    assert_eq!(client.balance(&g.b), -10);
}

#[test]
fn unknown_member_has_no_balance() {
    let g = setup();
    let stranger = Address::generate(&g.env);
    assert!(g.client().try_balance(&stranger).is_err());
}
