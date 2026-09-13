//! Reusable investment tracker library.
//!
//! The library owns domain behaviour and persistence. Applications such as the
//! command-line client or a future HTTP server should translate their inputs to
//! the request types in this module and avoid issuing SQL directly.

mod import;
mod rates;
mod store;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

pub use import::{ImportedAccount, ImportedValuation, SpreadsheetImporter};
pub use rates::{ExchangeRateProvider, HttpExchangeRateProvider};
pub use store::{migrate, open_database};

#[derive(Debug, Error)]
pub enum Error {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("account '{0}' was not found")]
    AccountNotFound(String),
    #[error("account name '{0}' already exists")]
    DuplicateAccount(String),
    #[error("subcategory '{0}' was not found")]
    SubcategoryNotFound(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("import error: {0}")]
    Import(String),
    #[error("external service error: {0}")]
    External(String),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tracker {
    pub name: String,
    pub storage_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub balance_gbp: f64,
    pub ownership_percent: f64,
    pub subcategory: String,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Valuation {
    pub date: String,
    pub balance_gbp: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Totals {
    pub assets_gbp: f64,
    pub liabilities_gbp: f64,
    pub net_gbp: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subcategory {
    pub id: i64,
    pub name: String,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAccount {
    pub name: String,
    pub kind: String,
    pub balance_gbp: f64,
    pub ownership_percent: f64,
    pub subcategory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAccount {
    pub balance_gbp: f64,
    pub subcategory: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountQuery {
    pub include_archived: bool,
}

/// High-level synchronous API for tracker operations.
pub struct TrackerService {
    connection: rusqlite::Connection,
    tracker: Tracker,
}

impl TrackerService {
    pub fn open(path: impl AsRef<Path>, name: impl Into<String>) -> Result<Self> {
        let path = path.as_ref().to_string_lossy().into_owned();
        let connection = open_database(path.as_ref())?;
        let tracker = Tracker {
            name: name.into(),
            storage_path: path,
        };
        Ok(Self {
            connection,
            tracker,
        })
    }

    pub fn tracker(&self) -> &Tracker {
        &self.tracker
    }

    pub fn create_account(&self, request: CreateAccount) -> Result<Account> {
        validate_amount(request.balance_gbp)?;
        validate_ownership(request.ownership_percent)?;
        let name = request.name.trim();
        if name.is_empty() {
            return Err(Error::Validation("account name cannot be empty".into()));
        }
        let exists: Option<String> = self.connection.query_row(
            "SELECT name FROM tracker_accounts WHERE lower(trim(name)) = lower(trim(?1)) LIMIT 1",
            [name], |row| row.get(0)).optional()?;
        if exists.is_some() {
            return Err(Error::DuplicateAccount(name.to_string()));
        }
        let id = unique_id(&self.connection, name)?;
        let date = today();
        let tx = self.connection.unchecked_transaction()?;
        tx.execute("INSERT INTO tracker_accounts (id,name,kind,balance_gbp,ownership_percent,subcategory,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![id, name, request.kind, request.balance_gbp, request.ownership_percent, request.subcategory, date])?;
        tx.execute("INSERT INTO tracker_valuations (account_id,valuation_date,balance_gbp,source) VALUES (?1,?2,?3,'manual')", params![id, date, request.balance_gbp])?;
        tx.commit()?;
        self.account(&id)
    }

    pub fn accounts(&self, query: AccountQuery) -> Result<Vec<Account>> {
        let sql = if query.include_archived {
            "SELECT id,name,kind,balance_gbp,ownership_percent,subcategory,archived FROM tracker_accounts ORDER BY name"
        } else {
            "SELECT id,name,kind,balance_gbp,ownership_percent,subcategory,archived FROM tracker_accounts WHERE archived=0 ORDER BY name"
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map([], |row| {
            Ok(Account {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                balance_gbp: row.get(3)?,
                ownership_percent: row.get(4)?,
                subcategory: row.get(5)?,
                archived: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn account(&self, id: &str) -> Result<Account> {
        self.connection.query_row("SELECT id,name,kind,balance_gbp,ownership_percent,subcategory,archived FROM tracker_accounts WHERE id=?1", [id], |row| Ok(Account { id: row.get(0)?, name: row.get(1)?, kind: row.get(2)?, balance_gbp: row.get(3)?, ownership_percent: row.get(4)?, subcategory: row.get(5)?, archived: row.get(6)? })).optional()?.ok_or_else(|| Error::AccountNotFound(id.to_string()))
    }

    pub fn update_account(&self, id: &str, request: UpdateAccount) -> Result<Account> {
        validate_amount(request.balance_gbp)?;
        let _ = self.account(id)?;
        let date = today();
        let tx = self.connection.unchecked_transaction()?;
        tx.execute("UPDATE tracker_accounts SET balance_gbp=?1,subcategory=?2,updated_at=?3 WHERE id=?4 AND archived=0", params![request.balance_gbp, request.subcategory, date, id])?;
        if tx.changes() == 0 {
            return Err(Error::AccountNotFound(id.to_string()));
        }
        tx.execute("INSERT INTO tracker_valuations (account_id,valuation_date,balance_gbp,source) VALUES (?1,?2,?3,'manual')", params![id, date, request.balance_gbp])?;
        tx.commit()?;
        self.account(id)
    }

    pub fn set_archived(&self, id: &str, archived: bool) -> Result<Account> {
        let changed = self.connection.execute(
            "UPDATE tracker_accounts SET archived=?1 WHERE id=?2",
            params![archived, id],
        )?;
        if changed == 0 {
            return Err(Error::AccountNotFound(id.to_string()));
        }
        self.account(id)
    }

    pub fn totals(&self) -> Result<Totals> {
        let (assets, liabilities) = self.connection.query_row("SELECT COALESCE(SUM(CASE WHEN kind='liability' THEN 0 ELSE balance_gbp*ownership_percent/100 END),0), COALESCE(SUM(CASE WHEN kind='liability' THEN balance_gbp*ownership_percent/100 ELSE 0 END),0) FROM tracker_accounts WHERE archived=0", [], |row| Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?)))?;
        Ok(Totals {
            assets_gbp: assets,
            liabilities_gbp: liabilities,
            net_gbp: assets - liabilities,
        })
    }

    pub fn valuations(&self, id: &str) -> Result<Vec<Valuation>> {
        let _ = self.account(id)?;
        let mut statement = self.connection.prepare("SELECT valuation_date,balance_gbp,source FROM tracker_valuations WHERE account_id=?1 ORDER BY valuation_date,id")?;
        let rows = statement.query_map([id], |row| {
            Ok(Valuation {
                date: row.get(0)?,
                balance_gbp: row.get(1)?,
                source: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn subcategories(&self, include_archived: bool) -> Result<Vec<Subcategory>> {
        let sql = if include_archived {
            "SELECT id,name,archived FROM subcategories ORDER BY name"
        } else {
            "SELECT id,name,archived FROM subcategories WHERE archived=0 ORDER BY name"
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map([], |row| {
            Ok(Subcategory {
                id: row.get(0)?,
                name: row.get(1)?,
                archived: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn add_subcategory(&self, name: &str) -> Result<Subcategory> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Validation("subcategory name cannot be empty".into()));
        }
        self.connection
            .execute("INSERT INTO subcategories (name) VALUES (?1)", [name])?;
        self.connection
            .query_row(
                "SELECT id,name,archived FROM subcategories WHERE name=?1",
                [name],
                |row| {
                    Ok(Subcategory {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        archived: row.get(2)?,
                    })
                },
            )
            .map_err(Error::from)
    }

    pub fn archive_subcategory(&self, name: &str) -> Result<()> {
        if self
            .connection
            .execute("UPDATE subcategories SET archived=1 WHERE name=?1", [name])?
            == 0
        {
            return Err(Error::SubcategoryNotFound(name.into()));
        }
        Ok(())
    }

    pub fn rename_subcategory(&self, old: &str, new: &str) -> Result<()> {
        if self.connection.execute(
            "UPDATE subcategories SET name=?1 WHERE name=?2",
            params![new.trim(), old],
        )? == 0
        {
            return Err(Error::SubcategoryNotFound(old.into()));
        }
        Ok(())
    }

    pub fn import_accounts(&self, accounts: &[ImportedAccount]) -> Result<usize> {
        let tx = self.connection.unchecked_transaction()?;
        for account in accounts {
            tx.execute("INSERT INTO tracker_accounts (id,name,kind,balance_gbp,ownership_percent,subcategory,archived,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind,balance_gbp=excluded.balance_gbp,ownership_percent=excluded.ownership_percent,subcategory=excluded.subcategory,archived=excluded.archived,updated_at=excluded.updated_at", params![account.id, account.name, account.kind, account.balance_gbp, account.ownership_percent, account.tag, account.archived, today()])?;
            tx.execute(
                "DELETE FROM tracker_valuations WHERE account_id=?1",
                [&account.id],
            )?;
            for valuation in &account.valuations {
                tx.execute("INSERT INTO tracker_valuations (account_id,valuation_date,balance_gbp,source) VALUES (?1,?2,?3,?4)", params![account.id, valuation.date, valuation.balance_gbp, valuation.source])?;
            }
        }
        tx.commit()?;
        Ok(accounts.len())
    }
}

fn validate_amount(value: f64) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        Err(Error::Validation(
            "amount must be finite and non-negative".into(),
        ))
    } else {
        Ok(())
    }
}
fn validate_ownership(value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        Err(Error::Validation(
            "ownership must be between 0 and 100 percent".into(),
        ))
    } else {
        Ok(())
    }
}
fn unique_id(connection: &rusqlite::Connection, name: &str) -> Result<String> {
    let base = slugify(name);
    let mut id = base.clone();
    let mut suffix = 2;
    while connection.query_row(
        "SELECT COUNT(*) FROM tracker_accounts WHERE id=?1",
        [&id],
        |row| row.get::<_, i64>(0),
    )? > 0
    {
        id = format!("{base}_{suffix}");
        suffix += 1;
    }
    Ok(id)
}
pub fn slugify(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}
pub fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_secs() as i64
        / 86_400;
    civil_date_from_days(days)
}
fn civil_date_from_days(days: i64) -> String {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_lifecycle_and_totals() {
        let service = TrackerService::open(":memory:", "test").unwrap();
        let account = service
            .create_account(CreateAccount {
                name: "ISA".into(),
                kind: "investment".into(),
                balance_gbp: 100.0,
                ownership_percent: 50.0,
                subcategory: "general".into(),
            })
            .unwrap();
        assert_eq!(service.totals().unwrap().assets_gbp, 50.0);
        service
            .update_account(
                &account.id,
                UpdateAccount {
                    balance_gbp: 200.0,
                    subcategory: "general".into(),
                },
            )
            .unwrap();
        assert_eq!(service.valuations(&account.id).unwrap().len(), 2);
        service.set_archived(&account.id, true).unwrap();
        assert_eq!(service.totals().unwrap().net_gbp, 0.0);
    }

    #[test]
    fn rejects_duplicate_names_case_insensitively() {
        let service = TrackerService::open(":memory:", "test").unwrap();
        let request = CreateAccount {
            name: "Savings".into(),
            kind: "savings".into(),
            balance_gbp: 1.0,
            ownership_percent: 100.0,
            subcategory: "general".into(),
        };
        service.create_account(request.clone()).unwrap();
        let duplicate = CreateAccount {
            name: " savings ".into(),
            ..request
        };
        assert!(matches!(
            service.create_account(duplicate),
            Err(Error::DuplicateAccount(_))
        ));
    }
}
