use serde::Serialize;
use sqlx::{Error, Row, Sqlite, SqlitePool, migrate::MigrateDatabase};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Server {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub port: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PingResult {
    pub id: i64,
    pub server_id: i64,
    pub pinged_at: String,
    pub online: bool,

    // frontend expects: player_count
    #[serde(rename = "player_count")]
    pub players_online: Option<i64>,

    pub players_max: Option<i64>,
    pub version: Option<String>,
    pub motd: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AdminUser {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
}

impl Database {
    /// Initialize the database connection pool, run migrations, and configure performance settings.
    pub async fn init(db_url: &str) -> Result<Self, Error> {
        // 1. Create database file if it doesn't exist
        if !Sqlite::database_exists(db_url).await.unwrap_or(false) {
            println!("Creating database file at: {}", db_url);
            Sqlite::create_database(db_url).await?;
        }

        // 2. Connect to the database
        let pool = SqlitePool::connect(db_url).await?;

        // 3. PERFORMANCE: Enable WAL Mode (Write-Ahead Logging)
        // This allows concurrent reads and writes, preventing the UI from freezing
        // while the background pinger is writing data.
        sqlx::query("PRAGMA journal_mode = WAL;")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL;")
            .execute(&pool)
            .await?;

        let db = Self { pool };

        // 4. Ensure schema exists
        db.run_migrations().await?;

        // 5. Seed default data if empty
        db.seed_default_server().await?;

        Ok(db)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    async fn run_migrations(&self) -> Result<(), Error> {
        // servers table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS servers (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                address     TEXT NOT NULL,
                port        INTEGER NOT NULL DEFAULT 25565,
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // ping_results table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS ping_results (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                server_id       INTEGER NOT NULL,
                pinged_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                online          INTEGER NOT NULL,
                latency_ms      INTEGER,
                players_online  INTEGER,
                players_max     INTEGER,
                version         TEXT,
                motd            TEXT,
                FOREIGN KEY (server_id) REFERENCES servers(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // PERFORMANCE: Index for faster graph loading
        // We frequently query by server_id and sort by date.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_ping_results_server_date 
            ON ping_results(server_id, pinged_at DESC);
            "#,
        )
        .execute(&self.pool)
        .await?;

        // admin_users table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS admin_users (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                username      TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // admin_sessions table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS admin_sessions (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                admin_id      INTEGER NOT NULL,
                session_token TEXT NOT NULL UNIQUE,
                created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                FOREIGN KEY (admin_id) REFERENCES admin_users(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn seed_default_server(&self) -> Result<(), Error> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM servers")
            .fetch_one(&self.pool)
            .await?;

        let count: i64 = row.try_get("count")?;

        if count == 0 {
            sqlx::query(
                r#"
                INSERT INTO servers (name, address, port)
                VALUES (?, ?, ?)
                "#,
            )
            .bind("Local test server")
            .bind("localhost")
            .bind(25565_i64)
            .execute(&self.pool)
            .await?;

            println!("Inserted default server (localhost:25565)");
        }

        Ok(())
    }

    // --- MAINTENANCE ---

    /// Deletes ping history older than `days` to keep database size manageable.
    /*
        /// Should be implemented however I just want more data to test it properly

        pub async fn cleanup_old_pings(&self, days: i64) -> Result<u64, Error> {
            let res = sqlx::query(
                r#"DELETE FROM ping_results WHERE pinged_at < date('now', '-' || ? || ' days')"#,
            )
            .bind(days)
            .execute(&self.pool)
            .await?;

            Ok(res.rows_affected())
        }
    */
    // --- QUERIES ---
    pub async fn insert_server(&self, name: &str, address: &str, port: i64) -> Result<i64, Error> {
        let res = sqlx::query("INSERT INTO servers (name, address, port) VALUES (?, ?, ?)")
            .bind(name)
            .bind(address)
            .bind(port)
            .execute(&self.pool)
            .await?;
        Ok(res.last_insert_rowid())
    }

    pub async fn delete_server(&self, id: i64) -> Result<u64, Error> {
        let res = sqlx::query("DELETE FROM servers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    pub async fn list_servers(&self) -> Result<Vec<Server>, Error> {
        sqlx::query_as::<_, Server>(
            "SELECT id, name, address, port, created_at FROM servers ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_server_by_id(&self, id: i64) -> Result<Option<Server>, Error> {
        sqlx::query_as::<_, Server>(
            "SELECT id, name, address, port, created_at FROM servers WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn get_last_ping_for_server(
        &self,
        server_id: i64,
    ) -> Result<Option<PingResult>, Error> {
        sqlx::query_as::<_, PingResult>(
            r#"
            SELECT id, server_id, pinged_at, online, players_online, players_max, version, motd
            FROM ping_results
            WHERE server_id = ?
            ORDER BY pinged_at DESC
            LIMIT 1
            "#,
        )
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn get_pings_subset(
        &self,
        server_id: i64,
        since_id: Option<i64>,
        seconds_ago: Option<u64>,
    ) -> Result<Vec<PingResult>, Error> {
        let mut sql = String::from(
            r#"
            SELECT id, server_id, pinged_at, online, players_online, players_max, version, motd
            FROM ping_results
            WHERE server_id = ?
            "#,
        );

        // If we only want new data (Incremental update)
        if let Some(_) = since_id {
            sql.push_str(" AND id > ?");
        }

        // If we are fetching a specific range (Day/Week/Month)
        if let Some(sec) = seconds_ago {
            // SQLite specific date math
            sql.push_str(&format!(
                " AND pinged_at >= datetime('now', '-{} seconds')",
                sec
            ));
        }

        sql.push_str(" ORDER BY pinged_at ASC"); // We want oldest to newest for the graph

        let mut query = sqlx::query_as::<_, PingResult>(&sql).bind(server_id);

        if let Some(sid) = since_id {
            query = query.bind(sid);
        }

        query.fetch_all(&self.pool).await
    }

    pub async fn insert_ping_result(
        &self,
        server_id: i64,
        online: bool,
        latency_ms: Option<i64>,
        players_online: Option<i64>,
        players_max: Option<i64>,
        version: Option<&str>,
        motd: Option<&str>,
    ) -> Result<i64, Error> {
        let res = sqlx::query(
            r#"
            INSERT INTO ping_results (server_id, online, latency_ms, players_online, players_max, version, motd)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
            .bind(server_id)
            .bind(if online { 1 } else { 0 })
            .bind(latency_ms)
            .bind(players_online)
            .bind(players_max)
            .bind(version)
            .bind(motd)
            .execute(&self.pool)
            .await?;
        Ok(res.last_insert_rowid())
    }

    /*
    pub async fn list_ping_results_for_server(
        &self,
        server_id: i64,
    ) -> Result<Vec<PingResult>, Error> {
        // Limit history to last 144 points to prevent frontend lag if data grows huge
        sqlx::query_as::<_, PingResult>(
            r#"
            SELECT id, server_id, pinged_at, online, players_online, players_max, version, motd
            FROM ping_results
            WHERE server_id = ?
            ORDER BY pinged_at DESC
            LIMIT 144
            "#,
        )
        .bind(server_id)
        .fetch_all(&self.pool)
        .await
    }
    */

    // --- AUTH ---

    pub async fn ensure_admin_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<(), Error> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM admin_users WHERE username = ?")
            .bind(username)
            .fetch_one(&self.pool)
            .await?;

        if row.try_get::<i64, _>("count")? == 0 {
            sqlx::query("INSERT INTO admin_users (username, password_hash) VALUES (?, ?)")
                .bind(username)
                .bind(password_hash)
                .execute(&self.pool)
                .await?;
            println!("Created default admin user '{}'", username);
        }
        Ok(())
    }

    pub async fn get_admin_by_username(&self, username: &str) -> Result<Option<AdminUser>, Error> {
        sqlx::query_as::<_, AdminUser>(
            "SELECT id, username, password_hash, created_at FROM admin_users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn create_admin_session(
        &self,
        admin_id: i64,
        session_token: &str,
    ) -> Result<(), Error> {
        sqlx::query("INSERT INTO admin_sessions (admin_id, session_token) VALUES (?, ?)")
            .bind(admin_id)
            .bind(session_token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_admin_by_session_token(
        &self,
        session_token: &str,
    ) -> Result<Option<AdminUser>, Error> {
        sqlx::query_as::<_, AdminUser>(
            r#"
            SELECT u.id, u.username, u.password_hash, u.created_at
            FROM admin_sessions s
            JOIN admin_users u ON s.admin_id = u.id
            WHERE s.session_token = ?
            "#,
        )
        .bind(session_token)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn delete_session(&self, session_token: &str) -> Result<(), Error> {
        sqlx::query("DELETE FROM admin_sessions WHERE session_token = ?")
            .bind(session_token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
