// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The connection every repository on two engines queries through (DD15). Its own module because the
//! derive puts `MultiBackend` and friends next to it.

use std::any::Any;
use std::borrow::Cow;
use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

use diesel::QueryResult;
use diesel::SqliteConnection;
use diesel::connection::{
    CacheSize, Connection, ConnectionSealed, Instrumentation, LoadConnection, SimpleConnection, TransactionManager,
    TransactionManagerStatus,
};
use diesel::expression::QueryMetadata;
use diesel::internal::derives::multiconnection::MultiConnectionHelper;
use diesel::migration::MigrationConnection;
use diesel::pg::{FailedToLookupTypeError, Pg, PgMetadataCacheKey, PgMetadataLookup, PgTypeMetadata};
use diesel::query_builder::{Query, QueryFragment, QueryId};
use diesel::r2d2::R2D2Connection;
use diesel::result::ConnectionResult;
use diesel::sql_types::TypeMetadata;
use diesel_async::AsyncPgConnection;
use diesel_async::async_connection_wrapper::{AsyncConnectionWrapper, BlockOn};

type Wrapped = AsyncConnectionWrapper<AsyncPgConnection, PgRuntime>;
type WrappedTm = <Wrapped as Connection>::TransactionManager;

/// Postgres through tokio-postgres instead of libpq, behind Diesel's sync `Connection`: no C build, and
/// every query written for `PgConnection` runs unchanged. A newtype rather than diesel-async's wrapper
/// itself, because `MultiConnection` needs [`MultiConnectionHelper`] and the wrapper's metadata lookup
/// panics in `as_any`. Boxed: the runtime and statement cache would make every `AnyConnection` 500 bytes.
pub struct PgConnection(Box<Wrapped>);

impl ConnectionSealed for PgConnection {}

impl SimpleConnection for PgConnection {
    fn batch_execute(&mut self, query: &str) -> QueryResult<()> {
        self.0.batch_execute(query)
    }
}

impl Connection for PgConnection {
    type Backend = Pg;
    type TransactionManager = PgTransactionManager;

    fn establish(database_url: &str) -> ConnectionResult<Self> {
        Wrapped::establish(database_url).map(|c| Self(Box::new(c)))
    }

    fn execute_returning_count<T>(&mut self, source: &T) -> QueryResult<usize>
    where
        T: QueryFragment<Pg> + QueryId,
    {
        self.0.execute_returning_count(source)
    }

    fn transaction_state(&mut self) -> &mut <PgTransactionManager as TransactionManager<Self>>::TransactionStateData {
        self.0.transaction_state()
    }

    fn instrumentation(&mut self) -> &mut dyn Instrumentation {
        self.0.instrumentation()
    }

    fn set_instrumentation(&mut self, instrumentation: impl Instrumentation) {
        self.0.set_instrumentation(instrumentation);
    }

    fn set_prepared_statement_cache_size(&mut self, size: CacheSize) {
        self.0.set_prepared_statement_cache_size(size);
    }
}

impl LoadConnection for PgConnection {
    type Cursor<'conn, 'query> = <Wrapped as LoadConnection>::Cursor<'conn, 'query>;
    type Row<'conn, 'query> = <Wrapped as LoadConnection>::Row<'conn, 'query>;

    fn load<'conn, 'query, T>(&'conn mut self, source: T) -> QueryResult<Self::Cursor<'conn, 'query>>
    where
        T: Query + QueryFragment<Pg> + QueryId + 'query,
        Pg: QueryMetadata<T::SqlType>,
    {
        self.0.load(source)
    }
}

impl MigrationConnection for PgConnection {
    fn setup(&mut self) -> QueryResult<usize> {
        self.0.setup()
    }
}

impl R2D2Connection for PgConnection {
    fn ping(&mut self) -> QueryResult<()> {
        self.0.ping()
    }

    fn is_broken(&mut self) -> bool {
        self.0.is_broken()
    }
}

impl MultiConnectionHelper for PgConnection {
    // The wrapper's own lookup can't become `Any`; only user-defined Postgres types ever need one, and the
    // schemas here are portable to SQLite, so they have none. A ZST: the leak allocates nothing.
    fn to_any<'a>(_: &mut <Pg as TypeMetadata>::MetadataLookup) -> &mut (dyn Any + 'a) {
        Box::leak(Box::new(NoCustomTypes))
    }

    fn from_any(lookup: &mut dyn Any) -> Option<&mut <Pg as TypeMetadata>::MetadataLookup> {
        lookup
            .downcast_mut::<NoCustomTypes>()
            .map(|l| l as &mut dyn PgMetadataLookup)
    }
}

struct NoCustomTypes;

impl PgMetadataLookup for NoCustomTypes {
    fn lookup_type(&mut self, type_name: &str, schema: Option<&str>) -> PgTypeMetadata {
        let key = PgMetadataCacheKey::new(schema.map(Cow::Borrowed), Cow::Borrowed(type_name)).into_owned();
        PgTypeMetadata::from_result(Err(FailedToLookupTypeError::new(key)))
    }

    fn as_any<'a>(&mut self) -> &mut (dyn Any + 'a)
    where
        Self: 'a,
    {
        self
    }
}

/// diesel-async's transaction manager, re-aimed at the newtype.
pub struct PgTransactionManager;

impl TransactionManager<PgConnection> for PgTransactionManager {
    type TransactionStateData = <WrappedTm as TransactionManager<Wrapped>>::TransactionStateData;

    fn begin_transaction(conn: &mut PgConnection) -> QueryResult<()> {
        WrappedTm::begin_transaction(&mut *conn.0)
    }

    fn rollback_transaction(conn: &mut PgConnection) -> QueryResult<()> {
        WrappedTm::rollback_transaction(&mut *conn.0)
    }

    fn commit_transaction(conn: &mut PgConnection) -> QueryResult<()> {
        WrappedTm::commit_transaction(&mut *conn.0)
    }

    fn transaction_manager_status_mut(conn: &mut PgConnection) -> &mut TransactionManagerStatus {
        WrappedTm::transaction_manager_status_mut(&mut *conn.0)
    }

    fn is_broken_transaction_manager(conn: &mut PgConnection) -> bool {
        WrappedTm::is_broken_transaction_manager(&mut *conn.0)
    }
}

/// SQLite or Postgres, chosen by [`crate::Engine`], never by guessing from a URL: Diesel's
/// `establish` on this enum tries each variant in turn and hides the Postgres error (DF7 spike).
#[derive(diesel::MultiConnection)]
pub enum AnyConnection {
    Sqlite(SqliteConnection),
    Postgres(PgConnection),
}

impl AnyConnection {
    pub fn engine(&self) -> crate::Engine {
        match self {
            Self::Sqlite(_) => crate::Engine::Sqlite,
            Self::Postgres(_) => crate::Engine::Postgres,
        }
    }
}

/// How a [`PgConnection`] waits: its socket and driver task live on one runtime shared by every Postgres
/// connection, and the calling thread parks until the answer comes. So it blocks from any thread, a
/// tokio worker or `Database::open` inside an async fn included, as libpq did; borrowing the caller's
/// runtime instead would panic on a current-thread one, and on dropping the connection inside async code.
pub struct PgRuntime(tokio::runtime::Handle);

impl BlockOn for PgRuntime {
    fn block_on<F: Future>(&self, f: F) -> F::Output {
        // The handle makes the shared runtime own the sockets and the driver task `establish` spawns.
        let _context = self.0.enter();
        // Unconstrained: called from inside a tokio task, the caller's coop budget would run out and every
        // tokio channel or lock poll after that returns Pending forever, as the task never yields to refill it.
        park_on(tokio::task::unconstrained(f))
    }

    #[allow(clippy::expect_used)] // the trait gives no way to fail; only the OS refusing threads gets here
    fn get_runtime() -> Self {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("store-postgres")
                .enable_all()
                .build()
                .expect("the Postgres connections' runtime")
        });
        Self(runtime.handle().clone())
    }
}

/// Polls `f` on this thread, parking between wake-ups.
fn park_on<F: Future>(f: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = pin!(f);
    loop {
        if let Poll::Ready(out) = f.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::park();
    }
}
