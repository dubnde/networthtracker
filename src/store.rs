use crate::Result;
use rusqlite::Connection;
use std::fs;
use std::path::Path;

pub fn open_database(path: &Path) -> Result<Connection> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let connection = if path == Path::new(":memory:") {
        Connection::open_in_memory()?
    } else {
        Connection::open(path)?
    };
    migrate(&connection)?;
    Ok(connection)
}

pub fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL); INSERT INTO schema_version SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version); CREATE TABLE IF NOT EXISTS tracker_accounts (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, balance_gbp REAL NOT NULL CHECK(balance_gbp >= 0), ownership_percent REAL NOT NULL DEFAULT 100, subcategory TEXT NOT NULL DEFAULT '', archived INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tracker_valuations (id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL REFERENCES tracker_accounts(id), valuation_date TEXT NOT NULL, balance_gbp REAL NOT NULL, source TEXT NOT NULL); CREATE INDEX IF NOT EXISTS idx_tracker_valuations_account_date ON tracker_valuations(account_id, valuation_date); CREATE TABLE IF NOT EXISTS subcategories (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, archived INTEGER NOT NULL DEFAULT 0);")?;
    Ok(())
}
