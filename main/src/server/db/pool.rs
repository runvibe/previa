use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use either::Either;
use futures_core::future::BoxFuture;
use futures_core::stream::BoxStream;
use futures_util::TryStreamExt;
use sqlx::any::{AnyConnectOptions, AnyPoolOptions};
use sqlx::database::Database;
use sqlx::error::Error;
use sqlx::{Any, Describe, Execute, Executor, Pool, Transaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Sqlite,
    Postgres,
}

impl DatabaseKind {
    pub fn from_url(database_url: &str) -> Result<Self, String> {
        let scheme = database_url
            .split_once(':')
            .map(|(scheme, _)| scheme)
            .unwrap_or_default();
        match scheme {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            _ => Err(format!(
                "unsupported database URL scheme '{}'; expected postgres:// or postgresql:// for runtime, or sqlite:// for project transfer",
                scheme
            )),
        }
    }
}

#[derive(Clone)]
pub struct DbPool {
    pool: Pool<Any>,
    kind: DatabaseKind,
}

impl fmt::Debug for DbPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbPool").field("kind", &self.kind).finish()
    }
}

impl DbPool {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, sqlx::Error> {
        let kind = DatabaseKind::from_url(database_url)
            .map_err(|err| sqlx::Error::Configuration(err.into()))?;
        if kind != DatabaseKind::Postgres {
            return Err(sqlx::Error::Configuration(
                "SQLite is supported only for project import/export; DATABASE_URL must use Postgres"
                    .into(),
            ));
        }
        Self::connect_with_kind(database_url, max_connections, kind).await
    }

    pub(crate) async fn connect_transfer_sqlite(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, sqlx::Error> {
        let kind = DatabaseKind::from_url(database_url)
            .map_err(|err| sqlx::Error::Configuration(err.into()))?;
        if kind != DatabaseKind::Sqlite {
            return Err(sqlx::Error::Configuration(
                "project transfer database must use SQLite".into(),
            ));
        }
        Self::connect_with_kind(database_url, max_connections, kind).await
    }

    #[cfg(test)]
    pub(crate) async fn connect_test_sqlite(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, sqlx::Error> {
        Self::connect_transfer_sqlite(database_url, max_connections).await
    }

    async fn connect_with_kind(
        database_url: &str,
        max_connections: u32,
        kind: DatabaseKind,
    ) -> Result<Self, sqlx::Error> {
        sqlx::any::install_default_drivers();
        if kind == DatabaseKind::Sqlite {
            create_sqlite_file_if_missing(database_url)?;
        }
        let options = database_url.parse::<AnyConnectOptions>()?;
        let mut pool_options = AnyPoolOptions::new().max_connections(max_connections);
        if kind == DatabaseKind::Sqlite {
            pool_options = pool_options.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    Ok(())
                })
            });
        }
        let pool = pool_options.connect_with(options).await?;
        Ok(Self { pool, kind })
    }

    pub fn new(pool: Pool<Any>, kind: DatabaseKind) -> Self {
        Self { pool, kind }
    }

    pub fn kind(&self) -> DatabaseKind {
        self.kind
    }

    pub fn pool(&self) -> &Pool<Any> {
        &self.pool
    }

    pub async fn begin(&self) -> Result<Transaction<'_, Any>, sqlx::Error> {
        self.pool.begin().await
    }

    pub fn sql<'q>(&self, sql: &'q str) -> &'q str {
        match self.kind {
            DatabaseKind::Sqlite => sql,
            DatabaseKind::Postgres => postgres_sql(sql),
        }
    }

    pub fn query<'q>(
        &self,
        sql: &'q str,
    ) -> sqlx::query::Query<'q, Any, <Any as sqlx::Database>::Arguments<'q>> {
        sqlx::query::<Any>(self.sql(sql))
    }

    fn rewrite_sql<'q>(&self, sql: &'q str) -> Cow<'q, str> {
        match self.kind {
            DatabaseKind::Sqlite => Cow::Borrowed(sql),
            DatabaseKind::Postgres => Cow::Owned(rewrite_question_placeholders(sql)),
        }
    }

    fn rewrite_sql_for_statement<'q>(&self, sql: &'q str) -> &'q str {
        match self.rewrite_sql(sql) {
            Cow::Borrowed(sql) => sql,
            Cow::Owned(sql) => Box::leak(sql.into_boxed_str()),
        }
    }
}

impl<'p> Executor<'p> for &'p DbPool {
    type Database = Any;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        mut query: E,
    ) -> BoxStream<'e, Result<Either<<Any as Database>::QueryResult, <Any as Database>::Row>, Error>>
    where
        'p: 'e,
        E: 'q + Execute<'q, Self::Database>,
    {
        let sql = self.rewrite_sql(query.sql());
        let arguments = match query.take_arguments().map_err(Error::Encode) {
            Ok(arguments) => arguments.unwrap_or_default(),
            Err(error) => return Box::pin(futures_util::stream::once(async { Err(error) })),
        };
        let pool = self.pool.clone();

        Box::pin(async_stream::try_stream! {
            #[allow(deprecated)]
            let mut rows = sqlx::query_with::<Any, _>(sql.as_ref(), arguments).fetch_many(&pool);
            while let Some(row) = rows.try_next().await? {
                yield row;
            }
        })
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        mut query: E,
    ) -> BoxFuture<'e, Result<Option<<Any as Database>::Row>, Error>>
    where
        'p: 'e,
        E: 'q + Execute<'q, Self::Database>,
    {
        let sql = self.rewrite_sql(query.sql());
        let arguments = match query.take_arguments().map_err(Error::Encode) {
            Ok(arguments) => arguments.unwrap_or_default(),
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let pool = self.pool.clone();

        Box::pin(async move {
            sqlx::query_with::<Any, _>(sql.as_ref(), arguments)
                .fetch_optional(&pool)
                .await
        })
    }

    fn prepare_with<'e, 'q: 'e>(
        self,
        sql: &'q str,
        parameters: &'e [<Self::Database as Database>::TypeInfo],
    ) -> BoxFuture<'e, Result<<Self::Database as Database>::Statement<'q>, Error>>
    where
        'p: 'e,
    {
        let sql = self.rewrite_sql_for_statement(sql);
        let pool = self.pool.clone();

        Box::pin(async move { (&pool).prepare_with(sql, parameters).await })
    }

    fn describe<'e, 'q: 'e>(
        self,
        sql: &'q str,
    ) -> BoxFuture<'e, Result<Describe<Self::Database>, Error>>
    where
        'p: 'e,
    {
        let sql = self.rewrite_sql_for_statement(sql);
        let pool = self.pool.clone();

        Box::pin(async move { (&pool).describe(sql).await })
    }
}

fn create_sqlite_file_if_missing(database_url: &str) -> Result<(), sqlx::Error> {
    let Some(path) = database_url.strip_prefix("sqlite://") else {
        return Ok(());
    };
    let path = path.split_once('?').map(|(path, _)| path).unwrap_or(path);
    if path.is_empty() || path == ":memory:" {
        return Ok(());
    }

    let path = Path::new(path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|err| {
            sqlx::Error::Configuration(
                format!("failed to create sqlite database directory: {err}").into(),
            )
        })?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(|_| ())
        .map_err(|err| {
            sqlx::Error::Configuration(
                format!("failed to create sqlite database file: {err}").into(),
            )
        })
}

fn postgres_sql<'q>(sql: &'q str) -> &'q str {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(rewritten) = cache.lock().expect("sql cache lock").get(sql).copied() {
        return rewritten;
    }

    let rewritten = rewrite_question_placeholders(sql);
    let leaked = Box::leak(rewritten.into_boxed_str());
    cache
        .lock()
        .expect("sql cache lock")
        .insert(sql.to_owned(), leaked);
    leaked
}

fn rewrite_question_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut placeholder = 1usize;
    let mut chars = sql.chars().peekable();
    let mut in_single_quote = false;

    while let Some(ch) = chars.next() {
        if ch == '\'' {
            out.push(ch);
            if in_single_quote && matches!(chars.peek(), Some('\'')) {
                out.push(chars.next().expect("peeked quote"));
                continue;
            }
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '?' && !in_single_quote {
            out.push('$');
            out.push_str(&placeholder.to_string());
            placeholder += 1;
        } else {
            out.push(ch);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{DatabaseKind, rewrite_question_placeholders};

    #[test]
    fn detects_database_kind_from_url() {
        assert_eq!(
            DatabaseKind::from_url("sqlite://orchestrator.db").expect("sqlite"),
            DatabaseKind::Sqlite
        );
        assert_eq!(
            DatabaseKind::from_url("postgres://user:pass@localhost/db").expect("postgres"),
            DatabaseKind::Postgres
        );
        assert_eq!(
            DatabaseKind::from_url("postgresql://user:pass@localhost/db").expect("postgresql"),
            DatabaseKind::Postgres
        );
        assert!(DatabaseKind::from_url("mysql://localhost/db").is_err());
    }

    #[tokio::test]
    async fn rejects_sqlite_for_operational_database() {
        let error = super::DbPool::connect("sqlite::memory:", 1)
            .await
            .expect_err("operational SQLite must be rejected");
        assert!(error.to_string().contains("only for project import/export"));
    }

    #[test]
    fn rewrites_placeholders_for_postgres() {
        assert_eq!(
            rewrite_question_placeholders(
                "SELECT * FROM projects WHERE id = ? AND name = '?' AND updated_at_ms > ?"
            ),
            "SELECT * FROM projects WHERE id = $1 AND name = '?' AND updated_at_ms > $2"
        );
    }
}
