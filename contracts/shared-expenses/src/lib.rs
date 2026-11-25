#![no_std]

//! Shared expenses for roommates or family.
//!
//! Members register expenses paid by one of them and split between a set of
//! participants. The contract keeps a running balance per member:
//!   * positive balance  -> the member is owed money
//!   * negative balance  -> the member owes money
//! Balances always sum to zero. `settle_plan` computes a short list of
//! transfers that leaves everyone at zero, and `record_payment` registers a
//! transfer that was actually made.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Map, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotEnoughMembers = 3,
    DuplicateMember = 4,
    NotMember = 5,
    InvalidAmount = 6,
    NoParticipants = 7,
    DuplicateParticipant = 8,
    SelfPayment = 9,
    NothingOwed = 10,
    PaymentTooLarge = 11,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Expense {
    pub payer: Address,
    pub amount: i128,
    pub participants: Vec<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transfer {
    pub from: Address,
    pub to: Address,
    pub amount: i128,
}

#[contracttype]
pub enum DataKey {
    Members,
    Balances,
    Expenses,
}

#[contract]
pub struct SharedExpenses;

#[contractimpl]
impl SharedExpenses {
    /// Sets up the group once. Needs at least two distinct members.
    pub fn init(env: Env, members: Vec<Address>) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Members) {
            return Err(Error::AlreadyInitialized);
        }
        if members.len() < 2 {
            return Err(Error::NotEnoughMembers);
        }
        for i in 0..members.len() {
            for j in (i + 1)..members.len() {
                if members.get(i).unwrap() == members.get(j).unwrap() {
                    return Err(Error::DuplicateMember);
                }
            }
        }

        let mut balances: Map<Address, i128> = Map::new(&env);
        for m in members.iter() {
            balances.set(m, 0);
        }

        env.storage().instance().set(&DataKey::Members, &members);
        env.storage().instance().set(&DataKey::Balances, &balances);
        env.storage()
            .instance()
            .set(&DataKey::Expenses, &Vec::<Expense>::new(&env));
        Ok(())
    }

    /// Registers an expense paid by `payer` and split equally among
    /// `participants`. Any remainder of the division goes to the first
    /// participants, one stroop-unit each, so no amount is lost.
    /// The payer must authorize the call.
    pub fn add_expense(
        env: Env,
        payer: Address,
        amount: i128,
        participants: Vec<Address>,
    ) -> Result<(), Error> {
        payer.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if participants.is_empty() {
            return Err(Error::NoParticipants);
        }

        let members = get_members(&env)?;
        let mut balances = get_balances(&env)?;
        require_member(&members, &payer)?;

        for i in 0..participants.len() {
            let p = participants.get(i).unwrap();
            require_member(&members, &p)?;
            for j in (i + 1)..participants.len() {
                if p == participants.get(j).unwrap() {
                    return Err(Error::DuplicateParticipant);
                }
            }
        }

        let n = participants.len() as i128;
        let base = amount / n;
        let remainder = amount % n;

        let payer_balance = balances.get(payer.clone()).unwrap_or(0);
        balances.set(payer.clone(), payer_balance + amount);

        for i in 0..participants.len() {
            let p = participants.get(i).unwrap();
            let share = base + if (i as i128) < remainder { 1 } else { 0 };
            let current = balances.get(p.clone()).unwrap_or(0);
            balances.set(p, current - share);
        }

        let mut expenses = get_expenses(&env)?;
        expenses.push_back(Expense {
            payer,
            amount,
            participants,
        });

        env.storage().instance().set(&DataKey::Balances, &balances);
        env.storage().instance().set(&DataKey::Expenses, &expenses);
        Ok(())
    }

    /// Registers that `from` paid `amount` to `to` outside the contract.
    /// `from` must authorize the call. The payment cannot exceed what `from`
    /// owes nor what `to` is owed.
    pub fn record_payment(env: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if from == to {
            return Err(Error::SelfPayment);
        }

        let members = get_members(&env)?;
        require_member(&members, &from)?;
        require_member(&members, &to)?;

        let mut balances = get_balances(&env)?;
        let from_balance = balances.get(from.clone()).unwrap_or(0);
        let to_balance = balances.get(to.clone()).unwrap_or(0);

        if from_balance >= 0 {
            return Err(Error::NothingOwed);
        }
        if amount > -from_balance || amount > to_balance {
            return Err(Error::PaymentTooLarge);
        }

        balances.set(from.clone(), from_balance + amount);
        balances.set(to.clone(), to_balance - amount);
        env.storage().instance().set(&DataKey::Balances, &balances);
        Ok(())
    }

    /// Current balance of a member: positive means owed, negative means owes.
    pub fn balance(env: Env, member: Address) -> Result<i128, Error> {
        let members = get_members(&env)?;
        require_member(&members, &member)?;
        Ok(get_balances(&env)?.get(member).unwrap_or(0))
    }

    /// Minimal-ish list of transfers that would bring every balance to zero.
    /// Greedy: the largest creditor is paired with the largest debtor until
    /// nobody is left owing or owed. Creates at most `members - 1` transfers.
    /// Read-only: it does not change any balance.
    pub fn settle_plan(env: Env) -> Result<Vec<Transfer>, Error> {
        let members = get_members(&env)?;
        let mut balances = get_balances(&env)?;
        let mut plan: Vec<Transfer> = Vec::new(&env);

        loop {
            let mut creditor: Option<(Address, i128)> = None;
            let mut debtor: Option<(Address, i128)> = None;

            for m in members.iter() {
                let b = balances.get(m.clone()).unwrap_or(0);
                if b > 0 && creditor.as_ref().map_or(true, |(_, cb)| b > *cb) {
                    creditor = Some((m.clone(), b));
                } else if b < 0 && debtor.as_ref().map_or(true, |(_, db)| b < *db) {
                    debtor = Some((m.clone(), b));
                }
            }

            match (creditor, debtor) {
                (Some((c, cb)), Some((d, db))) => {
                    let amount = if cb < -db { cb } else { -db };
                    plan.push_back(Transfer {
                        from: d.clone(),
                        to: c.clone(),
                        amount,
                    });
                    balances.set(c.clone(), cb - amount);
                    balances.set(d.clone(), db + amount);
                }
                _ => break,
            }
        }

        Ok(plan)
    }

    pub fn members(env: Env) -> Result<Vec<Address>, Error> {
        get_members(&env)
    }

    pub fn expenses(env: Env) -> Result<Vec<Expense>, Error> {
        get_expenses(&env)
    }
}

fn get_members(env: &Env) -> Result<Vec<Address>, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Members)
        .ok_or(Error::NotInitialized)
}

fn get_balances(env: &Env) -> Result<Map<Address, i128>, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Balances)
        .ok_or(Error::NotInitialized)
}

fn get_expenses(env: &Env) -> Result<Vec<Expense>, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Expenses)
        .ok_or(Error::NotInitialized)
}

fn require_member(members: &Vec<Address>, addr: &Address) -> Result<(), Error> {
    for m in members.iter() {
        if m == *addr {
            return Ok(());
        }
    }
    Err(Error::NotMember)
}

#[cfg(test)]
mod test;
