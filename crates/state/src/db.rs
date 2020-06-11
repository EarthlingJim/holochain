//! Functionality for safely accessing LMDB database references.

use crate::{
    env::EnvironmentKind,
    error::{DatabaseError, DatabaseResult},
};
use holochain_keystore::KeystoreSender;
use holochain_types::universal_map::{Key as UmKey, UniversalMap};
use lazy_static::lazy_static;
use parking_lot::RwLock;
use rkv::{IntegerStore, MultiStore, Rkv, SingleStore, StoreOptions};
use std::collections::{hash_map, HashMap};
use std::path::{Path, PathBuf};

/// TODO This is incomplete
/// Enumeration of all databases needed by Holochain
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum DbName {
    /// Primary database: KV store of chain entries, keyed by address
    PrimaryChainPublicEntries,
    /// Primary database: KV store of chain entries, keyed by address
    PrimaryChainPrivateEntries,
    /// Primary database: KV store of chain headers, keyed by address
    PrimaryChainHeaders,
    /// Primary database: KVV store of chain metadata, storing relationships
    PrimaryMetadata,
    /// Primary database: Kv store of links
    PrimaryLinksMeta,
    /// int KV store storing the sequence of committed headers,
    /// most notably allowing access to the chain head
    ChainSequence,
    /// Cache database: KV store of chain entries, keyed by address
    CacheChainEntries,
    /// Cache database: KV store of chain headers, keyed by address
    CacheChainHeaders,
    /// Cache database: KVV store of chain metadata, storing relationships
    CacheMetadata,
    /// Cachedatabase: Kv store of links
    CacheLinksMeta,
    /// database which stores a single key-value pair, encoding the
    /// mutable state for the entire Conductor
    ConductorState,
    /// database that stores wasm bytecode
    Wasm,
    /// database to store the [DnaDef]
    DnaDef,
    /// KVV store to accumulate validation receipts for a published EntryHash
    ValidationReceipts,
}

impl std::fmt::Display for DbName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use DbName::*;
        match self {
            PrimaryChainPublicEntries => write!(f, "PrimaryChainPublicEntries"),
            PrimaryChainPrivateEntries => write!(f, "PrimaryChainPrivateEntries"),
            PrimaryChainHeaders => write!(f, "PrimaryChainHeaders"),
            PrimaryMetadata => write!(f, "PrimaryMetadata"),
            PrimaryLinksMeta => write!(f, "PrimaryLinksMeta"),
            ChainSequence => write!(f, "ChainSequence"),
            CacheChainEntries => write!(f, "CacheChainEntries"),
            CacheChainHeaders => write!(f, "CacheChainHeaders"),
            CacheMetadata => write!(f, "CacheMetadata"),
            CacheLinksMeta => write!(f, "CacheLinksMeta"),
            ConductorState => write!(f, "ConductorState"),
            Wasm => write!(f, "Wasm"),
            DnaDef => write!(f, "DnaDef"),
            ValidationReceipts => write!(f, "ValidationReceipts"),
        }
    }
}

impl DbName {
    /// Associates a [DbKind] to each [DbName]
    pub fn kind(&self) -> DbKind {
        use DbKind::*;
        use DbName::*;
        match self {
            PrimaryChainPublicEntries => Single,
            PrimaryChainPrivateEntries => Single,
            PrimaryChainHeaders => Single,
            PrimaryMetadata => Multi,
            PrimaryLinksMeta => Single,
            ChainSequence => SingleInt,
            CacheChainEntries => Single,
            CacheChainHeaders => Single,
            CacheMetadata => Multi,
            CacheLinksMeta => Single,
            ConductorState => Single,
            Wasm => Single,
            DnaDef => Single,
            ValidationReceipts => Multi,
        }
    }
}

/// The various "modes" of viewing LMDB databases
pub enum DbKind {
    /// Single-value KV with arbitrary keys, associated with [KvBuf]
    Single,
    /// Single-value KV with integer keys, associated with [IntKvBuf]
    SingleInt,
    /// Multi-value KV with arbitrary keys, associated with [KvvBuf]
    Multi,
}

/// A UniversalMap key used to access persisted database references.
/// The key type is DbName, the value can be one of the various `rkv`
/// database types
pub type DbKey<V> = UmKey<DbName, V>;

type DbMap = UniversalMap<DbName>;

lazy_static! {
    /// The key to access the ChainEntries database
    pub static ref PRIMARY_CHAIN_PUBLIC_ENTRIES: DbKey<SingleStore> =
    DbKey::<SingleStore>::new(DbName::PrimaryChainPublicEntries);
    /// The key to access the PrivateChainEntries database
    pub static ref PRIMARY_CHAIN_PRIVATE_ENTRIES: DbKey<SingleStore> =
    DbKey::<SingleStore>::new(DbName::PrimaryChainPrivateEntries);
    /// The key to access the ChainHeaders database
    pub static ref PRIMARY_CHAIN_HEADERS: DbKey<SingleStore> =
    DbKey::<SingleStore>::new(DbName::PrimaryChainHeaders);
    /// The key to access the Metadata database
    pub static ref PRIMARY_SYSTEM_META: DbKey<MultiStore> = DbKey::new(DbName::PrimaryMetadata);
    /// The key to access the links database
    pub static ref PRIMARY_LINKS_META: DbKey<SingleStore> = DbKey::new(DbName::PrimaryLinksMeta);
    /// The key to access the ChainSequence database
    pub static ref CHAIN_SEQUENCE: DbKey<IntegerStore<u32>> = DbKey::new(DbName::ChainSequence);
    /// The key to access the ChainEntries database
    pub static ref CACHE_CHAIN_ENTRIES: DbKey<SingleStore> =
    DbKey::<SingleStore>::new(DbName::CacheChainEntries);
    /// The key to access the ChainHeaders database
    pub static ref CACHE_CHAIN_HEADERS: DbKey<SingleStore> =
    DbKey::<SingleStore>::new(DbName::CacheChainHeaders);
    /// The key to access the Metadata database
    pub static ref CACHE_SYSTEM_META: DbKey<MultiStore> = DbKey::new(DbName::CacheMetadata);
    /// The key to access the cache links database
    pub static ref CACHE_LINKS_META: DbKey<SingleStore> = DbKey::new(DbName::CacheLinksMeta);
    /// The key to access the ConductorState database
    pub static ref CONDUCTOR_STATE: DbKey<SingleStore> = DbKey::new(DbName::ConductorState);
    /// The key to access the Wasm database
    pub static ref WASM: DbKey<SingleStore> = DbKey::new(DbName::Wasm);
    /// The key to access the DnaDef database
    pub static ref DNA_DEF: DbKey<SingleStore> = DbKey::new(DbName::DnaDef);
    /// The key to access the ValidationReceipts database
    pub static ref VALIDATION_RECEIPTS: DbKey<MultiStore> = DbKey::new(DbName::ValidationReceipts);
}

lazy_static! {
    static ref DB_MAP_MAP: RwLock<HashMap<PathBuf, DbMap>> = RwLock::new(HashMap::new());
}

/// Get access to the singleton database manager ([GetDb]),
/// in order to access individual LMDB databases
pub(super) fn initialize_databases(rkv: &Rkv, kind: &EnvironmentKind) -> DatabaseResult<()> {
    let mut dbmap = DB_MAP_MAP.write();
    let path = rkv.path().to_owned();
    match dbmap.entry(path.clone()) {
        hash_map::Entry::Occupied(_) => {
            return Err(DatabaseError::EnvironmentDoubleInitialized(path))
        }
        hash_map::Entry::Vacant(e) => e.insert({
            let mut um = UniversalMap::new();
            register_databases(&rkv, kind, &mut um)?;
            um
        }),
    };
    Ok(())
}

pub(super) fn get_db<V: 'static + Copy + Send + Sync>(
    path: &Path,
    key: &'static DbKey<V>,
) -> DatabaseResult<V> {
    let dbmap = DB_MAP_MAP.read();
    let um: &DbMap = dbmap
        .get(path)
        .ok_or_else(|| DatabaseError::EnvironmentMissing(path.into()))?;
    let db = *um
        .get(key)
        .ok_or_else(|| DatabaseError::StoreNotInitialized(key.key().clone()))?;
    Ok(db)
}

fn register_databases(env: &Rkv, kind: &EnvironmentKind, um: &mut DbMap) -> DatabaseResult<()> {
    match kind {
        EnvironmentKind::Cell(_) => {
            register_db(env, um, &*PRIMARY_CHAIN_PUBLIC_ENTRIES)?;
            register_db(env, um, &*PRIMARY_CHAIN_PRIVATE_ENTRIES)?;
            register_db(env, um, &*PRIMARY_CHAIN_HEADERS)?;
            register_db(env, um, &*PRIMARY_SYSTEM_META)?;
            register_db(env, um, &*PRIMARY_LINKS_META)?;
            register_db(env, um, &*CHAIN_SEQUENCE)?;
            register_db(env, um, &*CACHE_CHAIN_ENTRIES)?;
            register_db(env, um, &*CACHE_CHAIN_HEADERS)?;
            register_db(env, um, &*CACHE_SYSTEM_META)?;
            register_db(env, um, &*CACHE_LINKS_META)?;
            register_db(env, um, &*VALIDATION_RECEIPTS)?;
        }
        EnvironmentKind::Conductor => {
            register_db(env, um, &*CONDUCTOR_STATE)?;
        }
        EnvironmentKind::Wasm => {
            register_db(env, um, &*WASM)?;
            register_db(env, um, &*DNA_DEF)?;
        }
    }
    Ok(())
}

fn register_db<V: 'static + Send + Sync>(
    env: &Rkv,
    um: &mut DbMap,
    key: &DbKey<V>,
) -> DatabaseResult<()> {
    let db_name = key.key();
    let db_str = format!("{}", db_name);
    let _ = match db_name.kind() {
        DbKind::Single => um.insert(
            key.with_value_type(),
            env.open_single(db_str.as_str(), StoreOptions::create())?,
        ),
        DbKind::SingleInt => um.insert(
            key.with_value_type(),
            env.open_integer::<&str, u32>(db_str.as_str(), StoreOptions::create())?,
        ),
        DbKind::Multi => {
            let mut opts = StoreOptions::create();

            // This is needed for the optional put flag NO_DUP_DATA on KvvBuf.
            // As far as I can tell, if we are not using NO_DUP_DATA, it will
            // only affect the sorting of the values in case there are dups,
            // which should be ok for our usage.
            //
            // NOTE - see:
            // https://github.com/mozilla/rkv/blob/0.10.4/src/env.rs#L122-L131
            //
            // Aparently RKV already sets this flag, but it's not mentioned
            // in the docs anywhere. We're going to set it too, just in case
            // it is removed out from under us at some point in the future.
            opts.flags.set(rkv::DatabaseFlags::DUP_SORT, true);

            um.insert(
                key.with_value_type(),
                env.open_multi(db_str.as_str(), opts)?,
            )
        }
    };
    Ok(())
}

/// GetDb allows access to the UniversalMap which stores the heterogeneously typed
/// LMDB Database references.
pub trait GetDb {
    /// Access an LMDB database environment stored in our static registrar.
    fn get_db<V: 'static + Copy + Send + Sync>(&self, key: &'static DbKey<V>) -> DatabaseResult<V>;
    /// Get a KeystoreSender to communicate with the Keystore task for this environment
    fn keystore(&self) -> KeystoreSender;
}
