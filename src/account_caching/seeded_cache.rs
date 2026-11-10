//! A seed-once, read-many [`SharedAccountsCache`].
//!
//! [`SeededCache`] holds a small, fixed set of accounts assembled up front
//! (the accounts a venue names in `requested_ctx()`) and is then read many
//! times by `quote()` without ever being written again. It is what the test
//! suite hands venues as `ctx`.

use std::collections::HashMap;

use solana_account::{Account, AccountSharedData};
use solana_pubkey::Pubkey;

use super::{AccountCacheError, SharedAccountsCache};

/// Seed-once, read-many account cache. See the [module docs](self).
#[derive(Debug, Default, Clone)]
pub struct SeededCache {
    accounts: HashMap<Pubkey, Option<AccountSharedData>>,
}

impl SeededCache {
    /// Build a cache holding exactly `accounts`. A `None` value records that
    /// the account is known to be absent; a read returns `Ok(None)` either way.
    pub fn from_accounts<I>(accounts: I) -> Self
    where
        I: IntoIterator<Item = (Pubkey, Option<Account>)>,
    {
        Self {
            accounts: accounts
                .into_iter()
                .map(|(key, account)| (key, account.map(AccountSharedData::from)))
                .collect(),
        }
    }

    /// Insert or replace entries after construction.
    pub fn insert_keys(&mut self, keys: &[(Pubkey, Option<Account>)]) {
        for (key, account) in keys {
            self.accounts
                .insert(*key, account.clone().map(AccountSharedData::from));
        }
    }

    /// Number of seeded entries, counting keys seeded as known-absent.
    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

impl SharedAccountsCache for SeededCache {
    fn get_account_shared(
        &self,
        pubkey: &Pubkey,
    ) -> Result<Option<AccountSharedData>, AccountCacheError> {
        Ok(self.accounts.get(pubkey).cloned().flatten())
    }

    fn get_accounts_shared(
        &self,
        pubkeys: &[Pubkey],
    ) -> Result<Vec<Option<AccountSharedData>>, AccountCacheError> {
        pubkeys.iter().map(|k| self.get_account_shared(k)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_account::ReadableAccount;

    #[test]
    fn returns_seeded_accounts_and_none_for_the_rest() {
        let present = Pubkey::new_unique();
        let absent = Pubkey::new_unique();
        let unknown = Pubkey::new_unique();
        let account = Account {
            lamports: 7,
            data: vec![1, 2, 3],
            owner: Pubkey::new_unique(),
            executable: false,
            rent_epoch: 0,
        };
        let cache = SeededCache::from_accounts([(present, Some(account.clone())), (absent, None)]);

        let got = cache.get_account_shared(&present).unwrap().unwrap();
        assert_eq!(got.lamports(), 7);
        assert_eq!(got.data(), &[1, 2, 3]);
        assert_eq!(Account::from(got), account);
        assert!(cache.get_account_shared(&absent).unwrap().is_none());
        assert!(cache.get_account_shared(&unknown).unwrap().is_none());

        let many = cache
            .get_accounts_shared(&[unknown, present, absent])
            .unwrap();
        assert!(many[0].is_none() && many[1].is_some() && many[2].is_none());
        assert_eq!(cache.len(), 2);
    }
}
