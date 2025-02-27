use std::{
    collections::{btree_map, BTreeMap},
    sync::Arc,
    vec::Vec,
};

// use reth_optimism_chainspec::OpChainSpec;
// use reth_optimism_forks::OpHardfork;
// use revm::{
//     db::{
//         states::{
//             bundle_state::BundleRetention, plain_account::PlainStorage, BundleState, CacheAccount, TransitionAccount,
//             TransitionState,
//         },
//         EmptyDB, PlainAccount, WrapDatabaseRef,
//     },
//     DBBox, DatabaseRef,
// };
// use revm_interpreter::primitives::{
//     db::Database, hash_map, Account, AccountInfo, Address, Bytecode, HashMap, B256, BLOCK_HASH_HISTORY, U256,
// };
// use revm_primitives::{address, b256, hex, Bytes, EvmState};

/// State of blockchain.
///
/// State clear flag is set inside CacheState and by default it is enabled.
/// If you want to disable it use `set_state_clear_flag` function.
#[derive(Debug)]
pub struct State<DB> {
    /// Cached state contains both changed from evm execution and cached/loaded account/storages
    /// from database. This allows us to have only one layer of cache where we can fetch data.
    /// Additionally we can introduce some preloading of data from database.
    pub cache: CacheState,
    /// Optional database that we use to fetch data from. If database is not present, we will
    /// return not existing account and storage.
    ///
    /// Note: It is marked as Send so database can be shared between threads.
    pub database: DB,
    /// Block state, it aggregates transactions transitions into one state.
    // / Build reverts and state that gets applied to the state.
    pub transition_state: Option<TransitionState>,
    /// After block is finishes we merge those changes inside bundle.
    /// Bundle is used to update database and create changesets.
    /// Bundle state can be set on initialization if we want to use preloaded bundle.
    pub bundle_state: BundleState,
    /// Addition layer that is going to be used to fetched values before fetching values
    /// from database.
    ///
    /// Bundle is the main output of the state execution and this allows setting previous bundle
    /// and using its values for execution.
    pub use_preloaded_bundle: bool,
    /// If EVM asks for block hash we will first check if they are found here.
    /// and then ask the database.
    ///
    /// This map can be used to give different values for block hashes if in case
    /// The fork block is different or some blocks are not saved inside database.
    pub block_hashes: BTreeMap<u64, B256>,
}
impl<Db> State<Db> {
    pub fn new(db: Db) -> Self {
        State::builder()
            .with_database(db)
            .with_bundle_update()
            .build()
    }

    pub fn new_with_bundle(db: Db, bundle: BundleState) -> Self {
        State::builder()
            .with_database(db)
            .with_bundle_update()
            .with_bundle_prestate(bundle)
            .build()
    }

    /// Apply evm transitions to transition state.
    pub fn apply_transition(&mut self, transitions: Vec<(Address, TransitionAccount)>) {
        // add transition to transition state.
        if let Some(s) = self.transition_state.as_mut() {
            s.add_transitions(transitions)
        }
    }

    /// reset all the caches/temporary fields
    pub fn reset(&mut self) {
        self.cache.accounts.clear();
        self.cache.contracts.clear();

        if let Some(s) = self.transition_state.as_mut() {
            s.transitions.clear();
        }

        self.bundle_state.state.clear();
        self.bundle_state.contracts.clear();
        self.bundle_state.reverts.clear();
        self.bundle_state.state_size = 0;
        self.bundle_state.reverts_size = 0;
        self.use_preloaded_bundle = false;
        self.block_hashes.clear();
    }
}

// Have ability to call State::builder without having to specify the type.
impl State<EmptyDB> {
    /// Return the builder that build the State.
    pub fn builder() -> StateBuilder<EmptyDB> {
        StateBuilder::default()
    }
}

impl<DB: DatabaseRef> State<DB> {
    /// Returns the size hint for the inner bundle state.
    /// See [BundleState::size_hint] for more info.
    pub fn bundle_size_hint(&self) -> usize {
        self.bundle_state.size_hint()
    }

    /// Iterate over received balances and increment all account balances.
    /// If account is not found inside cache state it will be loaded from database.
    ///
    /// Update will create transitions for all accounts that are updated.
    ///
    /// Like [CacheAccount::increment_balance], this assumes that incremented balances are not
    /// zero, and will not overflow once incremented. If using this to implement withdrawals, zero
    /// balances must be filtered out before calling this function.
    pub fn increment_balances(
        &mut self,
        balances: impl IntoIterator<Item = (Address, u128)>,
    ) -> Result<(), DB::Error> {
        // make transition and update cache state
        let mut transitions = Vec::new();
        for (address, balance) in balances {
            if balance == 0 {
                continue;
            }
            let original_account = self.load_cache_account(address)?;
            transitions.push((
                address,
                original_account
                    .increment_balance(balance)
                    .expect("Balance is not zero"),
            ))
        }
        // append transition
        if let Some(s) = self.transition_state.as_mut() {
            s.add_transitions(transitions)
        }
        Ok(())
    }

    /// Drain balances from given account and return those values.
    ///
    /// It is used for DAO hardfork state change to move values from given accounts.
    pub fn drain_balances(
        &mut self,
        addresses: impl IntoIterator<Item = Address>,
    ) -> Result<Vec<u128>, DB::Error> {
        // make transition and update cache state
        let mut transitions = Vec::new();
        let mut balances = Vec::new();
        for address in addresses {
            let original_account = self.load_cache_account(address)?;
            let (balance, transition) = original_account.drain_balance();
            balances.push(balance);
            transitions.push((address, transition))
        }
        // append transition
        if let Some(s) = self.transition_state.as_mut() {
            s.add_transitions(transitions)
        }
        Ok(balances)
    }

    /// State clear EIP-161 is enabled in Spurious Dragon hardfork.
    pub fn set_state_clear_flag(&mut self, has_state_clear: bool) {
        self.cache.set_state_clear_flag(has_state_clear);
    }

    pub fn insert_not_existing(&mut self, address: Address) {
        self.cache.insert_not_existing(address)
    }

    pub fn insert_account(&mut self, address: Address, info: AccountInfo) {
        self.cache.insert_account(address, info)
    }

    pub fn insert_account_with_storage(
        &mut self,
        address: Address,
        info: AccountInfo,
        storage: PlainStorage,
    ) {
        self.cache
            .insert_account_with_storage(address, info, storage)
    }

    /// Take all transitions and merge them inside bundle state.
    /// This action will create final post state and all reverts so that
    /// we at any time revert state of bundle to the state before transition
    /// is applied.
    pub fn merge_transitions(&mut self, retention: BundleRetention) {
        if let Some(transition_state) = self.transition_state.as_mut().map(TransitionState::take) {
            self.bundle_state
                .apply_transitions_and_create_reverts(transition_state, retention);
        }
    }

    /// Get a mutable reference to the [`CacheAccount`] for the given address.
    /// If the account is not found in the cache, it will be loaded from the
    /// database and inserted into the cache.
    pub fn load_cache_account(&mut self, address: Address) -> Result<&mut CacheAccount, DB::Error> {
        match self.cache.accounts.entry(address) {
            hash_map::Entry::Vacant(entry) => {
                if self.use_preloaded_bundle {
                    // load account from bundle state
                    if let Some(account) =
                        self.bundle_state.account(&address).cloned().map(Into::into)
                    {
                        return Ok(entry.insert(account));
                    }
                }
                // if not found in bundle, load it from database
                let info = self.database.basic_ref(address)?;
                let account = match info {
                    None => CacheAccount::new_loaded_not_existing(),
                    Some(acc) if acc.is_empty() => {
                        CacheAccount::new_loaded_empty_eip161(HashMap::default())
                    }
                    Some(acc) => CacheAccount::new_loaded(acc, HashMap::default()),
                };
                Ok(entry.insert(account))
            }
            hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
        }
    }

    // TODO make cache aware of transitions dropping by having global transition counter.
    /// Takes the [`BundleState`] changeset from the [`State`], replacing it
    /// with an empty one.
    ///
    /// This will not apply any pending [`TransitionState`]. It is recommended
    /// to call [`State::merge_transitions`] before taking the bundle.
    ///
    /// If the `State` has been built with the
    /// [`StateBuilder::with_bundle_prestate`] option, the pre-state will be
    /// taken along with any changes made by [`State::merge_transitions`].
    pub fn take_bundle(&mut self) -> BundleState {
        core::mem::take(&mut self.bundle_state)
    }

    pub fn commit_ref(&mut self, evm_state: &HashMap<Address, Account>) {
        for a in evm_state.keys() {
            let _ = self.load_cache_account(*a);
        }
        let transitions = self.cache.apply_evm_state(evm_state);
        self.apply_transition(transitions);
    }
}

impl<DB: DatabaseRef> Database for State<DB> {
    type Error = DB::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.load_cache_account(address).map(|a| a.account_info())
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        let res = match self.cache.contracts.entry(code_hash) {
            hash_map::Entry::Occupied(entry) => Ok(entry.get().clone()),
            hash_map::Entry::Vacant(entry) => {
                if self.use_preloaded_bundle {
                    if let Some(code) = self.bundle_state.contracts.get(&code_hash) {
                        entry.insert(code.clone());
                        return Ok(code.clone());
                    }
                }
                // if not found in bundle ask database
                let code = self.database.code_by_hash_ref(code_hash)?;
                entry.insert(code.clone());
                Ok(code)
            }
        };
        res
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // Account is guaranteed to be loaded.
        // Note that storage from bundle is already loaded with account.
        if let Some(account) = self.cache.accounts.get_mut(&address) {
            // account will always be some, but if it is not, U256::ZERO will be returned.
            let is_storage_known = account.status.is_storage_known();
            Ok(account
                .account
                .as_mut()
                .map(|account| match account.storage.entry(index) {
                    hash_map::Entry::Occupied(entry) => Ok(*entry.get()),
                    hash_map::Entry::Vacant(entry) => {
                        // if account was destroyed or account is newly built
                        // we return zero and don't ask database.
                        let value = if is_storage_known {
                            U256::ZERO
                        } else {
                            self.database.storage_ref(address, index)?
                        };
                        entry.insert(value);
                        Ok(value)
                    }
                })
                .transpose()?
                .unwrap_or_default())
        } else {
            unreachable!("For accessing any storage account is guaranteed to be loaded beforehand")
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self.block_hashes.entry(number) {
            btree_map::Entry::Occupied(entry) => Ok(*entry.get()),
            btree_map::Entry::Vacant(entry) => {
                let ret = *entry.insert(self.database.block_hash_ref(number)?);

                // prune all hashes that are older then BLOCK_HASH_HISTORY
                let last_block = number.saturating_sub(BLOCK_HASH_HISTORY);
                while let Some(entry) = self.block_hashes.first_entry() {
                    if *entry.key() < last_block {
                        entry.remove();
                    } else {
                        break;
                    }
                }

                Ok(ret)
            }
        }
    }
}

// impl<DB: DatabaseRef> DatabaseCommit for State<DB> {
//     fn commit(&mut self, evm_state: HashMap<Address, Account>) {
//         self.commit_ref(&evm_state);
//     }
// }

impl<ExtDB: DatabaseRef> DatabaseRef for State<ExtDB> {
    type Error = ExtDB::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self.cache.accounts.get(&address) {
            Some(acc) => Ok(acc.account_info()),
            None => self.database.basic_ref(address),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self.cache.contracts.get(&code_hash) {
            Some(entry) => Ok(entry.clone()),
            None => self.database.code_by_hash_ref(code_hash),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self.cache.accounts.get(&address) {
            Some(acc_entry) => match acc_entry.storage_slot(index) {
                Some(entry) => Ok(entry),
                None => self.database.storage_ref(address, index),
            },
            None => self.database.storage_ref(address, index),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        match self.block_hashes.get(&number) {
            Some(entry) => Ok(*entry),
            None => self.database.block_hash_ref(number),
        }
    }
}

/// Allows building of State and initializing it with different options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateBuilder<DB> {
    /// Database that we use to fetch data from.
    database: DB,
    /// Enabled state clear flag that is introduced in Spurious Dragon hardfork.
    /// Default is true as spurious dragon happened long time ago.
    with_state_clear: bool,
    /// if there is prestate that we want to use.
    /// This would mean that we have additional state layer between evm and disk/database.
    with_bundle_prestate: Option<BundleState>,
    /// This will initialize cache to this state.
    with_cache_prestate: Option<CacheState>,
    /// Do we want to create reverts and update bundle state.
    /// Default is false.
    with_bundle_update: bool,
    /// Do we want to merge transitions in background.
    /// This will allows evm to continue executing.
    /// Default is false.
    with_background_transition_merge: bool,
    /// If we want to set different block hashes
    with_block_hashes: BTreeMap<u64, B256>,
}

impl StateBuilder<EmptyDB> {
    /// Create a new builder with an empty database.
    ///
    /// If you want to instantiate it with a specific database, use
    /// [`new_with_database`](Self::new_with_database).
    pub fn new() -> Self {
        Self::default()
    }
}

impl<DB: Database + Default> Default for StateBuilder<DB> {
    fn default() -> Self {
        Self::new_with_database(DB::default())
    }
}

impl<DB> StateBuilder<DB> {
    /// Create a new builder with the given database.
    pub fn new_with_database(database: DB) -> Self {
        Self {
            database,
            with_state_clear: true,
            with_cache_prestate: None,
            with_bundle_prestate: None,
            with_bundle_update: false,
            with_background_transition_merge: false,
            with_block_hashes: BTreeMap::new(),
        }
    }

    /// Set the database.
    pub fn with_database<ODB>(self, database: ODB) -> StateBuilder<ODB> {
        // cast to the different database,
        // Note that we return different type depending of the database NewDBError.
        StateBuilder {
            with_state_clear: self.with_state_clear,
            database,
            with_cache_prestate: self.with_cache_prestate,
            with_bundle_prestate: self.with_bundle_prestate,
            with_bundle_update: self.with_bundle_update,
            with_background_transition_merge: self.with_background_transition_merge,
            with_block_hashes: self.with_block_hashes,
        }
    }

    /// Takes [DatabaseRef] and wraps it with [WrapDatabaseRef].
    pub fn with_database_ref<ODB: DatabaseRef>(
        self,
        database: ODB,
    ) -> StateBuilder<WrapDatabaseRef<ODB>> {
        self.with_database(WrapDatabaseRef(database))
    }

    /// With boxed version of database.
    pub fn with_database_boxed<Error>(
        self,
        database: DBBox<'_, Error>,
    ) -> StateBuilder<DBBox<'_, Error>> {
        self.with_database(database)
    }

    /// By default state clear flag is enabled but for initial sync on mainnet
    /// we want to disable it so proper consensus changes are in place.
    pub fn without_state_clear(self) -> Self {
        Self {
            with_state_clear: false,
            ..self
        }
    }

    /// Allows setting prestate that is going to be used for execution.
    /// This bundle state will act as additional layer of cache.
    /// and State after not finding data inside StateCache will try to find it inside BundleState.
    ///
    /// On update Bundle state will be changed and updated.
    pub fn with_bundle_prestate(self, bundle: BundleState) -> Self {
        Self {
            with_bundle_prestate: Some(bundle),
            ..self
        }
    }

    /// Make transitions and update bundle state.
    ///
    /// This is needed option if we want to create reverts
    /// and getting output of changed states.
    pub fn with_bundle_update(self) -> Self {
        Self {
            with_bundle_update: true,
            ..self
        }
    }

    /// It will use different cache for the state. If set, it will ignore bundle prestate.
    /// and will ignore `without_state_clear` flag as cache contains its own state_clear flag.
    ///
    /// This is useful for testing.
    pub fn with_cached_prestate(self, cache: CacheState) -> Self {
        Self {
            with_cache_prestate: Some(cache),
            ..self
        }
    }

    /// Starts the thread that will take transitions and do merge to the bundle state
    /// in the background.
    pub fn with_background_transition_merge(self) -> Self {
        Self {
            with_background_transition_merge: true,
            ..self
        }
    }

    pub fn with_block_hashes(self, block_hashes: BTreeMap<u64, B256>) -> Self {
        Self {
            with_block_hashes: block_hashes,
            ..self
        }
    }

    pub fn build(mut self) -> State<DB> {
        let use_preloaded_bundle = if self.with_cache_prestate.is_some() {
            self.with_bundle_prestate = None;
            false
        } else {
            self.with_bundle_prestate.is_some()
        };
        State {
            cache: self
                .with_cache_prestate
                .unwrap_or_else(|| CacheState::new(self.with_state_clear)),
            database: self.database,
            transition_state: self.with_bundle_update.then(TransitionState::default),
            bundle_state: self.with_bundle_prestate.unwrap_or_default(),
            use_preloaded_bundle,
            block_hashes: self.with_block_hashes,
        }
    }
}

/// Cache state contains both modified and original values.
///
/// Cache state is main state that revm uses to access state.
/// It loads all accounts from database and applies revm output to it.
///
/// It generates transitions that is used to build BundleState.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheState {
    /// Block state account with account state.
    pub accounts: HashMap<Address, CacheAccount>,
    /// Created contracts.
    // TODO add bytecode counter for number of bytecodes added/removed.
    pub contracts: HashMap<B256, Bytecode>,
    /// Has EIP-161 state clear enabled (Spurious Dragon hardfork).
    pub has_state_clear: bool,
}

impl Default for CacheState {
    fn default() -> Self {
        Self::new(true)
    }
}

impl CacheState {
    /// New default state.
    pub fn new(has_state_clear: bool) -> Self {
        Self {
            accounts: HashMap::default(),
            contracts: HashMap::default(),
            has_state_clear,
        }
    }

    /// Set state clear flag. EIP-161.
    pub fn set_state_clear_flag(&mut self, has_state_clear: bool) {
        self.has_state_clear = has_state_clear;
    }

    /// Helper function that returns all accounts.
    ///
    /// Used inside tests to generate merkle tree.
    pub fn trie_account(&self) -> impl IntoIterator<Item = (Address, &PlainAccount)> {
        self.accounts.iter().filter_map(|(address, account)| {
            account
                .account
                .as_ref()
                .map(|plain_acc| (*address, plain_acc))
        })
    }

    /// Insert not existing account.
    pub fn insert_not_existing(&mut self, address: Address) {
        self.accounts
            .insert(address, CacheAccount::new_loaded_not_existing());
    }

    /// Insert Loaded (Or LoadedEmptyEip161 if account is empty) account.
    pub fn insert_account(&mut self, address: Address, info: AccountInfo) {
        let account = if !info.is_empty() {
            CacheAccount::new_loaded(info, HashMap::default())
        } else {
            CacheAccount::new_loaded_empty_eip161(HashMap::default())
        };
        self.accounts.insert(address, account);
    }

    /// Similar to `insert_account` but with storage.
    pub fn insert_account_with_storage(
        &mut self,
        address: Address,
        info: AccountInfo,
        storage: PlainStorage,
    ) {
        let account = if !info.is_empty() {
            CacheAccount::new_loaded(info, storage)
        } else {
            CacheAccount::new_loaded_empty_eip161(storage)
        };
        self.accounts.insert(address, account);
    }

    /// Apply output of revm execution and create account transitions that are used to build BundleState.
    pub fn apply_evm_state(&mut self, evm_state: &EvmState) -> Vec<(Address, TransitionAccount)> {
        let mut transitions = Vec::with_capacity(evm_state.len());
        for (address, account) in evm_state {
            if let Some(transition) = self.apply_account_state(address, account) {
                transitions.push((*address, transition));
            }
        }
        transitions
    }

    /// Apply updated account state to the cached account.
    /// Returns account transition if applicable.
    fn apply_account_state(
        &mut self,
        address: &Address,
        account: &Account,
    ) -> Option<TransitionAccount> {
        // not touched account are never changed.
        if !account.is_touched() {
            return None;
        }

        let this_account = self
            .accounts
            .get_mut(address)
            .expect("All accounts should be present inside cache");

        // If it is marked as selfdestructed inside revm
        // we need to changed state to destroyed.
        if account.is_selfdestructed() {
            return this_account.selfdestruct();
        }

        let is_created = account.is_created();
        let is_empty = account.is_empty();

        // transform evm storage to storage with previous value.
        let changed_storage = account
            .storage
            .iter()
            .filter(|(_, slot)| slot.is_changed())
            .map(|(key, slot)| (*key, slot.clone().into()))
            .collect();

        // Note: it can happen that created contract get selfdestructed in same block
        // that is why is_created is checked after selfdestructed
        //
        // Note: Create2 opcode (Petersburg) was after state clear EIP (Spurious Dragon)
        //
        // Note: It is possibility to create KECCAK_EMPTY contract with some storage
        // by just setting storage inside CRATE constructor. Overlap of those contracts
        // is not possible because CREATE2 is introduced later.
        if is_created {
            return Some(this_account.newly_created(account.info.clone(), changed_storage));
        }

        // Account is touched, but not selfdestructed or newly created.
        // Account can be touched and not changed.
        // And when empty account is touched it needs to be removed from database.
        // EIP-161 state clear
        if is_empty {
            if self.has_state_clear {
                // touch empty account.
                this_account.touch_empty_eip161()
            } else {
                // if account is empty and state clear is not enabled we should save
                // empty account.
                this_account.touch_create_pre_eip161(changed_storage)
            }
        } else {
            Some(this_account.change(account.info.clone(), changed_storage))
        }
    }
}
const CREATE_2_DEPLOYER_ADDR: Address = address!("13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2");

const CREATE_2_DEPLOYER_CODEHASH: B256 =
    b256!("b0550b5b431e30d38000efb7107aaa0ade03d48a7198a140edda9d27134468b2");

/// The raw bytecode of the create2 deployer contract.
const CREATE_2_DEPLOYER_BYTECODE: [u8; 1584] = hex!("6080604052600436106100435760003560e01c8063076c37b21461004f578063481286e61461007157806356299481146100ba57806366cfa057146100da57600080fd5b3661004a57005b600080fd5b34801561005b57600080fd5b5061006f61006a366004610327565b6100fa565b005b34801561007d57600080fd5b5061009161008c366004610327565b61014a565b60405173ffffffffffffffffffffffffffffffffffffffff909116815260200160405180910390f35b3480156100c657600080fd5b506100916100d5366004610349565b61015d565b3480156100e657600080fd5b5061006f6100f53660046103ca565b610172565b61014582826040518060200161010f9061031a565b7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe082820381018352601f90910116604052610183565b505050565b600061015683836102e7565b9392505050565b600061016a8484846102f0565b949350505050565b61017d838383610183565b50505050565b6000834710156101f4576040517f08c379a000000000000000000000000000000000000000000000000000000000815260206004820152601d60248201527f437265617465323a20696e73756666696369656e742062616c616e636500000060448201526064015b60405180910390fd5b815160000361025f576040517f08c379a000000000000000000000000000000000000000000000000000000000815260206004820181905260248201527f437265617465323a2062797465636f6465206c656e677468206973207a65726f60448201526064016101eb565b8282516020840186f5905073ffffffffffffffffffffffffffffffffffffffff8116610156576040517f08c379a000000000000000000000000000000000000000000000000000000000815260206004820152601960248201527f437265617465323a204661696c6564206f6e206465706c6f790000000000000060448201526064016101eb565b60006101568383305b6000604051836040820152846020820152828152600b8101905060ff815360559020949350505050565b61014e806104ad83390190565b6000806040838503121561033a57600080fd5b50508035926020909101359150565b60008060006060848603121561035e57600080fd5b8335925060208401359150604084013573ffffffffffffffffffffffffffffffffffffffff8116811461039057600080fd5b809150509250925092565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052604160045260246000fd5b6000806000606084860312156103df57600080fd5b8335925060208401359150604084013567ffffffffffffffff8082111561040557600080fd5b818601915086601f83011261041957600080fd5b81358181111561042b5761042b61039b565b604051601f82017fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe0908116603f011681019083821181831017156104715761047161039b565b8160405282815289602084870101111561048a57600080fd5b826020860160208301376000602084830101528095505050505050925092509256fe608060405234801561001057600080fd5b5061012e806100206000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c8063249cb3fa14602d575b600080fd5b603c603836600460b1565b604e565b60405190815260200160405180910390f35b60008281526020818152604080832073ffffffffffffffffffffffffffffffffffffffff8516845290915281205460ff16608857600060aa565b7fa2ef4600d742022d532d4747cb3547474667d6f13804902513b2ec01c848f4b45b9392505050565b6000806040838503121560c357600080fd5b82359150602083013573ffffffffffffffffffffffffffffffffffffffff8116811460ed57600080fd5b80915050925092905056fea26469706673582212205ffd4e6cede7d06a5daf93d48d0541fc68189eeb16608c1999a82063b666eb1164736f6c63430008130033a2646970667358221220fdc4a0fe96e3b21c108ca155438d37c9143fb01278a3c1d274948bad89c564ba64736f6c63430008130033");

pub fn ensure_create2_deployer<DB>(
    chain_spec: Arc<OpChainSpec>,
    timestamp: u64,
    db: &mut State<DB>,
) -> Result<(), DB::Error>
where
    DB: revm::DatabaseRef,
{
    // If the canyon hardfork is active at the current timestamp, and it was not active at the
    // previous block timestamp (heuristically, block time is not perfectly constant at 2s), and the
    // chain is an optimism chain, then we need to force-deploy the create2 deployer contract.
    if chain_spec.is_fork_active_at_timestamp(OpHardfork::Canyon, timestamp)
        && !chain_spec.is_fork_active_at_timestamp(OpHardfork::Canyon, timestamp.saturating_sub(2))
    {
        tracing::trace!(target: "evm", "Forcing create2 deployer contract deployment on Canyon transition");

        // Load the create2 deployer account from the cache.
        let acc = db.load_cache_account(CREATE_2_DEPLOYER_ADDR)?;

        // Update the account info with the create2 deployer codehash and bytecode.
        let mut acc_info = acc.account_info().unwrap_or_default();
        acc_info.code_hash = CREATE_2_DEPLOYER_CODEHASH;
        acc_info.code = Some(Bytecode::new_raw(Bytes::from_static(
            &CREATE_2_DEPLOYER_BYTECODE,
        )));

        // Convert the cache account back into a revm account and mark it as touched.
        let mut revm_acc: revm::primitives::Account = acc_info.into();
        revm_acc.mark_touch();

        // Commit the create2 deployer account to the database.
        db.commit_ref(&HashMap::from_iter([(CREATE_2_DEPLOYER_ADDR, revm_acc)]));
        return Ok(());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use revm::db::{
        states::{reverts::AccountInfoRevert, StorageSlot},
        AccountRevert, AccountStatus, BundleAccount, RevertToSlot,
    };
    use revm_interpreter::primitives::keccak256;

    use super::*;

    #[test]
    fn block_hash_cache() {
        let mut state = State::builder().build();
        state.block_hash(1u64).unwrap();
        state.block_hash(2u64).unwrap();

        let test_number = BLOCK_HASH_HISTORY + 2;

        let block1_hash = keccak256(U256::from(1).to_string().as_bytes());
        let block2_hash = keccak256(U256::from(2).to_string().as_bytes());
        let block_test_hash = keccak256(U256::from(test_number).to_string().as_bytes());

        assert_eq!(
            state.block_hashes,
            BTreeMap::from([(1, block1_hash), (2, block2_hash)])
        );

        state.block_hash(test_number).unwrap();
        assert_eq!(
            state.block_hashes,
            BTreeMap::from([(test_number, block_test_hash), (2, block2_hash)])
        );
    }

    /// Checks that if accounts is touched multiple times in the same block,
    /// then the old values from the first change are preserved and not overwritten.
    ///
    /// This is important because the state transitions from different transactions in the same block may see
    /// different states of the same account as the old value, but the revert should reflect the
    /// state of the account before the block.
    #[test]
    fn reverts_preserve_old_values() {
        let mut state = State::builder().with_bundle_update().build();

        let (slot1, slot2, slot3) = (U256::from(1), U256::from(2), U256::from(3));

        // Non-existing account for testing account state transitions.
        // [LoadedNotExisting] -> [Changed] (nonce: 1, balance: 1) -> [Changed] (nonce: 2) -> [Changed] (nonce: 3)
        let new_account_address = Address::from_slice(&[0x1; 20]);
        let new_account_created_info = AccountInfo {
            nonce: 1,
            balance: U256::from(1),
            ..Default::default()
        };
        let new_account_changed_info = AccountInfo {
            nonce: 2,
            ..new_account_created_info.clone()
        };
        let new_account_changed_info2 = AccountInfo {
            nonce: 3,
            ..new_account_changed_info.clone()
        };

        // Existing account for testing storage state transitions.
        let existing_account_address = Address::from_slice(&[0x2; 20]);
        let existing_account_initial_info = AccountInfo {
            nonce: 1,
            ..Default::default()
        };
        let existing_account_initial_storage = HashMap::<U256, U256>::from_iter([
            (slot1, U256::from(100)), // 0x01 => 100
            (slot2, U256::from(200)), // 0x02 => 200
        ]);
        let existing_account_changed_info = AccountInfo {
            nonce: 2,
            ..existing_account_initial_info.clone()
        };

        // A transaction in block 1 creates one account and changes an existing one.
        state.apply_transition(Vec::from([
            (
                new_account_address,
                TransitionAccount {
                    status: AccountStatus::InMemoryChange,
                    info: Some(new_account_created_info.clone()),
                    previous_status: AccountStatus::LoadedNotExisting,
                    previous_info: None,
                    ..Default::default()
                },
            ),
            (
                existing_account_address,
                TransitionAccount {
                    status: AccountStatus::InMemoryChange,
                    info: Some(existing_account_changed_info.clone()),
                    previous_status: AccountStatus::Loaded,
                    previous_info: Some(existing_account_initial_info.clone()),
                    storage: HashMap::from_iter([(
                        slot1,
                        StorageSlot::new_changed(
                            *existing_account_initial_storage.get(&slot1).unwrap(),
                            U256::from(1000),
                        ),
                    )]),
                    storage_was_destroyed: false,
                },
            ),
        ]));

        // A transaction in block 1 then changes the same account.
        state.apply_transition(Vec::from([(
            new_account_address,
            TransitionAccount {
                status: AccountStatus::InMemoryChange,
                info: Some(new_account_changed_info.clone()),
                previous_status: AccountStatus::InMemoryChange,
                previous_info: Some(new_account_created_info.clone()),
                ..Default::default()
            },
        )]));

        // Another transaction in block 1 then changes the newly created account yet again and modifies the storage in
        // an existing one.
        state.apply_transition(Vec::from([
            (
                new_account_address,
                TransitionAccount {
                    status: AccountStatus::InMemoryChange,
                    info: Some(new_account_changed_info2.clone()),
                    previous_status: AccountStatus::InMemoryChange,
                    previous_info: Some(new_account_changed_info),
                    storage: HashMap::from_iter([(
                        slot1,
                        StorageSlot::new_changed(U256::ZERO, U256::from(1)),
                    )]),
                    storage_was_destroyed: false,
                },
            ),
            (
                existing_account_address,
                TransitionAccount {
                    status: AccountStatus::InMemoryChange,
                    info: Some(existing_account_changed_info.clone()),
                    previous_status: AccountStatus::InMemoryChange,
                    previous_info: Some(existing_account_changed_info.clone()),
                    storage: HashMap::from_iter([
                        (
                            slot1,
                            StorageSlot::new_changed(U256::from(100), U256::from(1_000)),
                        ),
                        (
                            slot2,
                            StorageSlot::new_changed(
                                *existing_account_initial_storage.get(&slot2).unwrap(),
                                U256::from(2_000),
                            ),
                        ),
                        // Create new slot
                        (
                            slot3,
                            StorageSlot::new_changed(U256::ZERO, U256::from(3_000)),
                        ),
                    ]),
                    storage_was_destroyed: false,
                },
            ),
        ]));

        state.merge_transitions(BundleRetention::Reverts);
        let mut bundle_state = state.take_bundle();

        // The new account revert should be `DeleteIt` since this was an account creation.
        // The existing account revert should be reverted to its previous state.
        bundle_state.reverts.sort();
        assert_eq!(
            bundle_state.reverts.as_ref(),
            Vec::from([Vec::from([
                (
                    new_account_address,
                    AccountRevert {
                        account: AccountInfoRevert::DeleteIt,
                        previous_status: AccountStatus::LoadedNotExisting,
                        storage: HashMap::from_iter([(slot1, RevertToSlot::Some(U256::ZERO))]),
                        wipe_storage: false,
                    }
                ),
                (
                    existing_account_address,
                    AccountRevert {
                        account: AccountInfoRevert::RevertTo(existing_account_initial_info.clone()),
                        previous_status: AccountStatus::Loaded,
                        storage: HashMap::from_iter([
                            (
                                slot1,
                                RevertToSlot::Some(
                                    *existing_account_initial_storage.get(&slot1).unwrap()
                                )
                            ),
                            (
                                slot2,
                                RevertToSlot::Some(
                                    *existing_account_initial_storage.get(&slot2).unwrap()
                                )
                            ),
                            (slot3, RevertToSlot::Some(U256::ZERO))
                        ]),
                        wipe_storage: false,
                    }
                ),
            ])]),
            "The account or storage reverts are incorrect"
        );

        // The latest state of the new account should be: nonce = 3, balance = 1, code & code hash = None.
        // Storage: 0x01 = 1.
        assert_eq!(
            bundle_state.account(&new_account_address),
            Some(&BundleAccount {
                info: Some(new_account_changed_info2),
                original_info: None,
                status: AccountStatus::InMemoryChange,
                storage: HashMap::from_iter([(
                    slot1,
                    StorageSlot::new_changed(U256::ZERO, U256::from(1))
                )]),
            }),
            "The latest state of the new account is incorrect"
        );

        // The latest state of the existing account should be: nonce = 2.
        // Storage: 0x01 = 1000, 0x02 = 2000, 0x03 = 3000.
        assert_eq!(
            bundle_state.account(&existing_account_address),
            Some(&BundleAccount {
                info: Some(existing_account_changed_info),
                original_info: Some(existing_account_initial_info),
                status: AccountStatus::InMemoryChange,
                storage: HashMap::from_iter([
                    (
                        slot1,
                        StorageSlot::new_changed(
                            *existing_account_initial_storage.get(&slot1).unwrap(),
                            U256::from(1_000)
                        )
                    ),
                    (
                        slot2,
                        StorageSlot::new_changed(
                            *existing_account_initial_storage.get(&slot2).unwrap(),
                            U256::from(2_000)
                        )
                    ),
                    // Create new slot
                    (
                        slot3,
                        StorageSlot::new_changed(U256::ZERO, U256::from(3_000))
                    ),
                ]),
            }),
            "The latest state of the existing account is incorrect"
        );
    }

    /// Checks that the accounts and storages that are changed within the
    /// block and reverted to their previous state do not appear in the reverts.
    #[test]
    fn bundle_scoped_reverts_collapse() {
        let mut state = State::builder().with_bundle_update().build();

        // Non-existing account.
        let new_account_address = Address::from_slice(&[0x1; 20]);
        let new_account_created_info = AccountInfo {
            nonce: 1,
            balance: U256::from(1),
            ..Default::default()
        };

        // Existing account.
        let existing_account_address = Address::from_slice(&[0x2; 20]);
        let existing_account_initial_info = AccountInfo {
            nonce: 1,
            ..Default::default()
        };
        let existing_account_updated_info = AccountInfo {
            nonce: 1,
            balance: U256::from(1),
            ..Default::default()
        };

        // Existing account with storage.
        let (slot1, slot2) = (U256::from(1), U256::from(2));
        let existing_account_with_storage_address = Address::from_slice(&[0x3; 20]);
        let existing_account_with_storage_info = AccountInfo {
            nonce: 1,
            ..Default::default()
        };
        // A transaction in block 1 creates a new account.
        state.apply_transition(Vec::from([
            (
                new_account_address,
                TransitionAccount {
                    status: AccountStatus::InMemoryChange,
                    info: Some(new_account_created_info.clone()),
                    previous_status: AccountStatus::LoadedNotExisting,
                    previous_info: None,
                    ..Default::default()
                },
            ),
            (
                existing_account_address,
                TransitionAccount {
                    status: AccountStatus::Changed,
                    info: Some(existing_account_updated_info.clone()),
                    previous_status: AccountStatus::Loaded,
                    previous_info: Some(existing_account_initial_info.clone()),
                    ..Default::default()
                },
            ),
            (
                existing_account_with_storage_address,
                TransitionAccount {
                    status: AccountStatus::Changed,
                    info: Some(existing_account_with_storage_info.clone()),
                    previous_status: AccountStatus::Loaded,
                    previous_info: Some(existing_account_with_storage_info.clone()),
                    storage: HashMap::from_iter([
                        (
                            slot1,
                            StorageSlot::new_changed(U256::from(1), U256::from(10)),
                        ),
                        (slot2, StorageSlot::new_changed(U256::ZERO, U256::from(20))),
                    ]),
                    storage_was_destroyed: false,
                },
            ),
        ]));

        // Another transaction in block 1 destroys new account.
        state.apply_transition(Vec::from([
            (
                new_account_address,
                TransitionAccount {
                    status: AccountStatus::Destroyed,
                    info: None,
                    previous_status: AccountStatus::InMemoryChange,
                    previous_info: Some(new_account_created_info),
                    ..Default::default()
                },
            ),
            (
                existing_account_address,
                TransitionAccount {
                    status: AccountStatus::Changed,
                    info: Some(existing_account_initial_info),
                    previous_status: AccountStatus::Changed,
                    previous_info: Some(existing_account_updated_info),
                    ..Default::default()
                },
            ),
            (
                existing_account_with_storage_address,
                TransitionAccount {
                    status: AccountStatus::Changed,
                    info: Some(existing_account_with_storage_info.clone()),
                    previous_status: AccountStatus::Changed,
                    previous_info: Some(existing_account_with_storage_info.clone()),
                    storage: HashMap::from_iter([
                        (
                            slot1,
                            StorageSlot::new_changed(U256::from(10), U256::from(1)),
                        ),
                        (slot2, StorageSlot::new_changed(U256::from(20), U256::ZERO)),
                    ]),
                    storage_was_destroyed: false,
                },
            ),
        ]));

        state.merge_transitions(BundleRetention::Reverts);

        let mut bundle_state = state.take_bundle();
        bundle_state.reverts.sort();

        // both account info and storage are left as before transitions,
        // therefore there is nothing to revert
        assert_eq!(bundle_state.reverts.as_ref(), Vec::from([Vec::from([])]));
    }

    /// Checks that the behavior of selfdestruct within the block is correct.
    #[test]
    fn selfdestruct_state_and_reverts() {
        let mut state = State::builder().with_bundle_update().build();

        // Existing account.
        let existing_account_address = Address::from_slice(&[0x1; 20]);
        let existing_account_info = AccountInfo {
            nonce: 1,
            ..Default::default()
        };

        let (slot1, slot2) = (U256::from(1), U256::from(2));

        // Existing account is destroyed.
        state.apply_transition(Vec::from([(
            existing_account_address,
            TransitionAccount {
                status: AccountStatus::Destroyed,
                info: None,
                previous_status: AccountStatus::Loaded,
                previous_info: Some(existing_account_info.clone()),
                storage: HashMap::default(),
                storage_was_destroyed: true,
            },
        )]));

        // Existing account is re-created and slot 0x01 is changed.
        state.apply_transition(Vec::from([(
            existing_account_address,
            TransitionAccount {
                status: AccountStatus::DestroyedChanged,
                info: Some(existing_account_info.clone()),
                previous_status: AccountStatus::Destroyed,
                previous_info: None,
                storage: HashMap::from_iter([(
                    slot1,
                    StorageSlot::new_changed(U256::ZERO, U256::from(1)),
                )]),
                storage_was_destroyed: false,
            },
        )]));

        // Slot 0x01 is changed, but existing account is destroyed again.
        state.apply_transition(Vec::from([(
            existing_account_address,
            TransitionAccount {
                status: AccountStatus::DestroyedAgain,
                info: None,
                previous_status: AccountStatus::DestroyedChanged,
                previous_info: Some(existing_account_info.clone()),
                // storage change should be ignored
                storage: HashMap::default(),
                storage_was_destroyed: true,
            },
        )]));

        // Existing account is re-created and slot 0x02 is changed.
        state.apply_transition(Vec::from([(
            existing_account_address,
            TransitionAccount {
                status: AccountStatus::DestroyedChanged,
                info: Some(existing_account_info.clone()),
                previous_status: AccountStatus::DestroyedAgain,
                previous_info: None,
                storage: HashMap::from_iter([(
                    slot2,
                    StorageSlot::new_changed(U256::ZERO, U256::from(2)),
                )]),
                storage_was_destroyed: false,
            },
        )]));

        state.merge_transitions(BundleRetention::Reverts);

        let bundle_state = state.take_bundle();

        assert_eq!(
            bundle_state.state,
            HashMap::from_iter([(
                existing_account_address,
                BundleAccount {
                    info: Some(existing_account_info.clone()),
                    original_info: Some(existing_account_info.clone()),
                    storage: HashMap::from_iter([(
                        slot2,
                        StorageSlot::new_changed(U256::ZERO, U256::from(2))
                    )]),
                    status: AccountStatus::DestroyedChanged,
                }
            )])
        );

        assert_eq!(
            bundle_state.reverts.as_ref(),
            Vec::from([Vec::from([(
                existing_account_address,
                AccountRevert {
                    account: AccountInfoRevert::DoNothing,
                    previous_status: AccountStatus::Loaded,
                    storage: HashMap::from_iter([(slot2, RevertToSlot::Destroyed)]),
                    wipe_storage: true,
                }
            )])])
        )
    }
}
