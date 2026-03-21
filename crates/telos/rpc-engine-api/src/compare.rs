use std::collections::HashSet;
use std::fmt::Display;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_primitives::map::HashMap;
use revm::state::{Account, AccountInfo, AccountStatus, Bytecode, EvmStorageSlot};
use revm::{Database, DatabaseCommit, database::{State, TransitionAccount}};
use revm::database::AccountStatus as DBAccountStatus;
use revm_primitives::KECCAK_EMPTY;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};
use reth_storage_errors::provider::ProviderError;
use crate::structs::{TelosAccountStateTableRow, TelosAccountTableRow};

struct StateOverride {
    accounts: HashMap<Address, Account>
}

impl StateOverride {
    pub fn new() -> Self {
        StateOverride {
            accounts: HashMap::default()
        }
    }

    fn maybe_init_account<DB: Database> (&mut self, revm_db: &mut State<DB>, address: Address) {
        let maybe_acc = self.accounts.get_mut(&address);
        if maybe_acc.is_none() {
            let mut status = AccountStatus::LoadedAsNotExisting | AccountStatus::Touched;
            let info = match revm_db.basic(address) {
                Ok(maybe_info) => {
                    maybe_info.unwrap_or_default()
                },
                Err(_) => {
                    status = AccountStatus::Created | AccountStatus::Touched;
                    AccountInfo::default()
                }
            };

            self.accounts.insert(address, Account {
                original_info: Box::new(info.clone()),
                info,
                storage: Default::default(),
                status,
                transaction_id: 0,
            });
        }
    }

    pub fn override_account<DB: Database> (&mut self, revm_db: &mut State<DB>, telos_row: &TelosAccountTableRow) {
        self.maybe_init_account(revm_db, telos_row.address);
        let acc = self.accounts.get_mut(&telos_row.address).unwrap();
        acc.info.balance = telos_row.balance;
        acc.info.nonce = telos_row.nonce;
        if telos_row.code.len() > 0 {
            acc.info.code_hash = B256::from_slice(Sha256::digest(telos_row.code.as_ref()).as_slice());
            acc.info.code = Some(Bytecode::new_legacy(telos_row.code.clone()));
        } else {
            acc.info.code_hash = KECCAK_EMPTY;
            acc.info.code = None;
        }
    }

    pub fn override_balance<DB: Database> (&mut self, revm_db: &mut State<DB>, address: Address, balance: U256) {
        self.maybe_init_account(revm_db, address);
        let acc = self.accounts.get_mut(&address).unwrap();
        acc.info.balance = balance;
    }

    pub fn override_nonce<DB: Database> (&mut self, revm_db: &mut State<DB>, address: Address, nonce: u64) {
        self.maybe_init_account(revm_db, address);
        let acc = self.accounts.get_mut(&address).unwrap();
        acc.info.nonce = nonce;
    }

    pub fn override_code<DB: Database> (&mut self, revm_db: &mut State<DB>, address: Address, maybe_code: &Bytes) {
        self.maybe_init_account(revm_db, address);
        let acc = self.accounts.get_mut(&address).unwrap();
        if maybe_code.len() > 0 {
            acc.info.code_hash = B256::from_slice(Sha256::digest(maybe_code.as_ref()).as_slice());
            acc.info.code = Some(Bytecode::new_legacy(maybe_code.clone()));
        } else {
            acc.info.code_hash = KECCAK_EMPTY;
            acc.info.code = None;
        }
    }

    pub fn override_storage<DB: Database> (&mut self, revm_db: &mut State<DB>, address: Address, key: U256, new_val: U256, old_val: U256) {
        self.maybe_init_account(revm_db, address);
        let acc = self.accounts.get_mut(&address).unwrap();
        acc.storage.insert(key, EvmStorageSlot {
            original_value: old_val,
            present_value: new_val,
            transaction_id: 0,
            is_cold: false
        });
    }

    pub fn apply<DB: Database> (&self, revm_db: &mut State<DB>) {
        revm_db.commit(self.accounts.clone());
    }
}

macro_rules! maybe_panic {
    ($panic_mode:expr, $($arg:tt)*) => {
        if $panic_mode {
            panic!($($arg)*);
        } else {
            warn!($($arg)*);
        }
    };
}

/// This function compares the state diffs between revm and Telos EVM contract
pub fn compare_state_diffs<DB>(
    block_number: u64,
    revm_db: &mut State<DB>,
    revm_state_diffs: HashMap<Address, TransitionAccount>,
    statediffs_account: Vec<TelosAccountTableRow>,
    statediffs_accountstate: Vec<TelosAccountStateTableRow>,
    _new_addresses_using_create: Vec<(u64, U256)>,
    new_addresses_using_openwallet: Vec<(u64, U256)>,
    panic_mode: bool,
    do_storage: bool
) -> bool
where
    DB: Database,
    DB::Error: Into<ProviderError> + Display,
{
    if !revm_state_diffs.is_empty()
        || !statediffs_account.is_empty()
        || !statediffs_accountstate.is_empty()
    {
        debug!("{block_number} REVM State diffs: {:#?}", revm_state_diffs);
        debug!("{block_number} TEVM State diffs account: {:#?}", statediffs_account);
        debug!("{block_number} TEVM State diffs accountstate: {:#?}", statediffs_accountstate);
    }

    let mut state_override = StateOverride::new();

    let mut new_addresses_using_openwallet_hashset = HashSet::new();
    for row in &new_addresses_using_openwallet {
        new_addresses_using_openwallet_hashset.insert(Address::from_word(B256::from(row.1)));
    }

    let mut statediffs_account_hashmap = HashSet::new();
    for row in &statediffs_account {
        statediffs_account_hashmap.insert(row.address);
    }

    for row in &statediffs_account {
        // Skip if address is created using openwallet and is empty
        if new_addresses_using_openwallet_hashset.contains(&row.address) && row.balance == U256::ZERO && row.nonce == 0 && row.code.len() == 0 {
            continue;
        }
        // Skip if row is removed
        if row.removed {
            continue
        }
        if let Ok(revm_row) = revm_db.basic(row.address) {
            if let Some(unwrapped_revm_row) = revm_row {
                // Check balance inequality
                if unwrapped_revm_row.balance != row.balance {
                    maybe_panic!(panic_mode, "Difference in balance, address: {:?} - revm: {:?} - tevm: {:?}",row.address,unwrapped_revm_row.balance,row.balance);
                    state_override.override_balance(revm_db, row.address, row.balance);
                }
                // Check nonce inequality
                if unwrapped_revm_row.nonce != row.nonce {
                    maybe_panic!(panic_mode, "Difference in nonce, address: {:?} - revm: {:?} - tevm: {:?}",row.address,unwrapped_revm_row.nonce,row.nonce);
                    state_override.override_nonce(revm_db, row.address, row.nonce);
                }
                // Check code size inequality
                let maybe_revm_code = unwrapped_revm_row.clone().code;

                match maybe_revm_code {
                    None => {
                        if row.code.len() != 0 {
                            match revm_db.code_by_hash(unwrapped_revm_row.code_hash()) {
                                Ok(bytecode) => {
                                    if bytecode.len() != row.code.len() {
                                        maybe_panic!(panic_mode, "Difference in code size, address: {:?} - revm: {} - tevm: {}",row.address,bytecode.len(),row.code.len());
                                        state_override.override_code(revm_db, row.address, &row.code);
                                    }
                                }
                                Err(_) => {
                                    maybe_panic!(panic_mode, "Difference in code existence, error while fetching db, address: {:?} - revm: Err - tevm: {}",row.address,row.code.len());
                                    state_override.override_code(revm_db, row.address, &row.code);
                                }
                            }
                        }
                    }
                    Some(revm_bytecode) => {
                       match revm_bytecode {
                           Bytecode::LegacyAnalyzed(code) => {
                               if code.original_len() != row.code.len() {
                                   maybe_panic!(panic_mode, "Difference in legacy code size, address: {:?} - revm: {:?} - tevm: {:?}",row.address,code.original_len(),row.code.len());
                                   state_override.override_code(revm_db, row.address, &row.code);
                               }
                           }
                           Bytecode::Eip7702(_) => panic!("EIP7702 not implemented!")
                       }
                    }
                }
            } else {
                // Skip if address is empty on both sides
                if !(row.balance == U256::ZERO && row.nonce == 0 && row.code.len() == 0) {
                    if let Some(unwrapped_revm_state_diff) = revm_state_diffs.get(&row.address) {
                        if !(unwrapped_revm_state_diff.status == DBAccountStatus::Destroyed && row.nonce == 0 && row.balance == U256::ZERO && row.code.len() == 0) {
                            maybe_panic!(panic_mode, "A modified `account` table row was found on both revm state and revm state diffs, but seems to be destroyed on just one side, address: {:?}",row.address);
                            state_override.override_account(revm_db, &row);
                        }
                    } else {
                        maybe_panic!(panic_mode, "A modified `account` table row was found on revm state, but contains no information, address: {:?}",row.address);
                        state_override.override_account(revm_db, &row);
                    }
                }
            }
        } else {
            // Skip if address is empty on both sides
            if !(row.balance == U256::ZERO && row.nonce == 0 && row.code.len() == 0) {
                maybe_panic!(panic_mode, "A modified `account` table row was not found on revm state, address: {:?}",row.address);
                state_override.override_account(revm_db, &row);
            }
        }
    }
    if do_storage {
        for row in &statediffs_accountstate {
            if revm_db.cache.accounts.get_mut(&row.address).is_none() {
                let cached_account = revm_db.load_cache_account(row.address);
                match cached_account {
                    Ok(cached_account) => {
                        if cached_account.account.is_none() {
                            panic!("An account state modification was made for an account that is not in revm storage, address: {:?}", row.address);
                        }
                    },
                    Err(_) => {
                        panic!("An account state modification was made for an account that returned Err from load_cache_account, address: {:?}", row.address);
                    }
                }
            }
            if let Ok(revm_row) = revm_db.storage(row.address, row.key) {
                // The values should match, but if it is removed, then the revm value should be zero
                if revm_row != row.value {
                    if revm_row != U256::ZERO && row.removed == true {
                        maybe_panic!(panic_mode, "Difference in value on revm storage, removed on Telos, non-ZERO on revm, address: {:?}, key: {:?}, revm-value: {:?}, tevm-row: {:?}", row.address, row.key, revm_row, row);
                        state_override.override_storage(revm_db, row.address, row.key, U256::ZERO, revm_row);
                    }
                    if row.removed == false {
                        maybe_panic!(panic_mode, "Difference in value on revm storage, address: {:?}, key: {:?}, revm-value: {:?}, tevm-row: {:?}", row.address, row.key, revm_row, row);
                        state_override.override_storage(revm_db, row.address, row.key, row.value, revm_row);
                    }
                }
            } else {
                maybe_panic!(panic_mode, "Key was not found on revm storage, address: {:?}, key: {:?}",row.address,row.key);
                state_override.override_storage(revm_db, row.address, row.key, row.value, U256::ZERO);
            }
        }
    }

    for (address, account) in &revm_state_diffs {
        if let (Some(info),Some(previous_info)) = (account.info.clone(),account.previous_info.clone()) {
            if !(info.balance == previous_info.balance && info.nonce == previous_info.nonce && info.code_hash == previous_info.code_hash) {
                if statediffs_account_hashmap.get(address).is_none() {
                    panic!("A modified address was not found on tevm state diffs, address: {:?}",address);
                }
            }
        } else {
            if statediffs_account_hashmap.get(address).is_none() {
                panic!("A modified address was not found on tevm state diffs, info/previous_info were not Some, address: {:?}",address);
            }
        }
    }

    state_override.apply(revm_db);

    true
}
