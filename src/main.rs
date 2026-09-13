use calamine::{Reader, open_workbook_auto};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_SPREADSHEET_PATH: &str = "/Users/dchingalande/Library/Mobile Documents/com~apple~CloudDocs/Downloads/Worth_it_export.xlsx";
const SQLITE_PATH: &str = "data/investments.sqlite3";

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = arguments.first().map(String::as_str).unwrap_or("help");

    let result = match command {
        "init" => init_tracker(&arguments[1..]),
        "add" => add_account(),
        "list" => list_accounts(&arguments[1..]),
        "update" => update_account_balance(&arguments[1..]),
        "archive" => archive_account(&arguments[1..]),
        "subcategory" => manage_subcategories(&arguments[1..]),
        "total" => total_transactions(),
        "import" => import_spreadsheet(&arguments[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!(
            "unknown command '{other}'. Use 'init', 'add', 'list', 'update', 'archive', 'subcategory', 'total', or 'import'."
        )),
    };

    if let Err(error) = result {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TrackerConfig {
    name: String,
    storage_path: String,
}

fn init_tracker(arguments: &[String]) -> Result<(), String> {
    if arguments.len() != 1 || arguments[0].trim().is_empty() {
        return Err("usage: cargo run -- init <tracker-name>".to_string());
    }
    let name = arguments[0].trim().to_string();
    let slug = slugify(&name);
    let storage_path = format!("data/{slug}.sqlite3");
    if std::path::Path::new("data/tracker.json").exists() {
        return Err(
            "a tracker is already initialized; remove data/tracker.json before reinitializing"
                .to_string(),
        );
    }
    fs::create_dir_all("data").map_err(|error| format!("cannot create data directory: {error}"))?;
    let config = TrackerConfig { name, storage_path };
    let connection = Connection::open(&config.storage_path)
        .map_err(|error| format!("cannot create tracker storage: {error}"))?;
    create_tracker_schema(&connection)?;
    fs::write(
        "data/tracker.json",
        serde_json::to_string_pretty(&config).unwrap() + "\n",
    )
    .map_err(|error| format!("cannot write tracker configuration: {error}"))?;
    println!(
        "Initialized tracker '{}' in {}.",
        config.name, config.storage_path
    );
    Ok(())
}

fn tracker_connection() -> Result<(TrackerConfig, Connection), String> {
    let config_text = fs::read_to_string("data/tracker.json").map_err(|_| {
        "tracker is not initialized; run `cargo run -- init <name>` first".to_string()
    })?;
    let config: TrackerConfig = serde_json::from_str(&config_text)
        .map_err(|error| format!("invalid tracker configuration: {error}"))?;
    let connection = Connection::open(&config.storage_path)
        .map_err(|error| format!("cannot open tracker storage: {error}"))?;
    create_tracker_schema(&connection)?;
    Ok((config, connection))
}

fn tracker_storage_path() -> Option<String> {
    let config_text = fs::read_to_string("data/tracker.json").ok()?;
    let config = serde_json::from_str::<TrackerConfig>(&config_text).ok()?;
    Some(config.storage_path)
}

fn create_tracker_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS transactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transaction_type TEXT NOT NULL CHECK(transaction_type IN ('income', 'expense')),
            amount_gbp REAL NOT NULL CHECK(amount_gbp >= 0),
            description TEXT NOT NULL,
            subcategory TEXT NOT NULL,
            transaction_date TEXT NOT NULL,
            archived INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_transactions_date ON transactions(transaction_date);
        CREATE TABLE IF NOT EXISTS subcategories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            archived INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS tracker_accounts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            balance_gbp REAL NOT NULL CHECK(balance_gbp >= 0),
            ownership_percent REAL NOT NULL DEFAULT 100,
            subcategory TEXT NOT NULL DEFAULT '',
            archived INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tracker_valuations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT NOT NULL REFERENCES tracker_accounts(id),
            valuation_date TEXT NOT NULL,
            balance_gbp REAL NOT NULL,
            source TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tracker_valuations_account_date ON tracker_valuations(account_id, valuation_date);",
        )
        .map_err(|error| format!("cannot create tracker schema: {error}"))
}

fn add_account() -> Result<(), String> {
    let (_, connection) = tracker_connection()?;
    let name = required_prompt("Account name: ")?;
    ensure_unique_account_name(&connection, &name)?;
    let kind = prompt_default("Account type [investment]: ", "investment")?;
    let balance = prompt_amount("Current balance [£]: ")?;
    if balance < 0.0 {
        return Err("balance cannot be negative".to_string());
    }
    let subcategory = prompt_default("Subcategory [general]: ", "general")?;
    let id = unique_account_id(&connection, &name)?;
    connection.execute("INSERT INTO tracker_accounts (id, name, kind, balance_gbp, subcategory, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, name, kind, balance, subcategory, today()]).map_err(|error| format!("cannot save account: {error}"))?;
    connection.execute("INSERT INTO tracker_valuations (account_id, valuation_date, balance_gbp, source) VALUES (?1, ?2, ?3, 'manual')", params![id, today(), balance]).map_err(|error| format!("cannot save account history: {error}"))?;
    println!(
        "Added account '{name}' with {}.",
        format_currency("£", balance)
    );
    Ok(())
}

fn list_accounts(arguments: &[String]) -> Result<(), String> {
    let (_, connection) = tracker_connection()?;
    let include_archived = arguments.iter().any(|argument| argument == "--archived");
    let show_ids = arguments.iter().any(|argument| argument == "--ids");
    let query = if include_archived {
        "SELECT id, name, kind, balance_gbp, subcategory, archived FROM tracker_accounts ORDER BY name"
    } else {
        "SELECT id, name, kind, balance_gbp, subcategory, archived FROM tracker_accounts WHERE archived = 0 ORDER BY name"
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|error| format!("cannot query accounts: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, bool>(5)?,
            ))
        })
        .map_err(|error| format!("cannot list accounts: {error}"))?;
    if show_ids {
        println!(
            "{:<24} {:<28} {:>15}  {:<14} Subcategory",
            "ID", "Account", "Balance", "Type"
        );
    } else {
        println!(
            "{:<32} {:>15}  {:<14} Subcategory",
            "Account", "Balance", "Type"
        );
    }
    println!("{}", "-".repeat(if show_ids { 90 } else { 66 }));
    let mut count = 0;
    for row in rows {
        let (id, name, kind, balance, subcategory, archived) =
            row.map_err(|error| error.to_string())?;
        if show_ids {
            println!(
                "{id:<24} {name:<28} {:>15}  {kind:<14} {}{}",
                format_currency("£", balance),
                subcategory,
                if archived { " [archived]" } else { "" }
            );
        } else {
            println!(
                "{name:<32} {:>15}  {kind:<14} {}{}",
                format_currency("£", balance),
                subcategory,
                if archived { " [archived]" } else { "" }
            );
        }
        count += 1;
    }
    if count == 0 {
        println!("No accounts found. Use `cargo run -- add` to add one.");
    }
    let (assets, liabilities) = account_totals(&connection)?;
    println!("{}", "-".repeat(if show_ids { 90 } else { 66 }));
    println!("{:<30} {:>15}", "Assets", format_currency("£", assets));
    println!(
        "{:<30} {:>15}",
        "Liabilities",
        format_currency("£", liabilities)
    );
    println!(
        "{:<30} {:>15}",
        "Net balance",
        format_currency("£", assets - liabilities)
    );
    match fetch_gbp_usd_rate() {
        Ok(rate) => println!(
            "{:<30} {:>15}  (GBP/USD: {:.4})",
            "Net balance (USD)",
            format_currency("$", (assets - liabilities) * rate),
            rate
        ),
        Err(error) => println!("{:<30} unavailable ({error})", "Net balance (USD)"),
    }
    Ok(())
}

fn update_account_balance(arguments: &[String]) -> Result<(), String> {
    let (_, connection) = tracker_connection()?;
    let id = if let Some(id) = arguments.first() {
        id.clone()
    } else {
        choose_account_id(&connection)?
    };
    let current = connection.query_row("SELECT name, balance_gbp, subcategory FROM tracker_accounts WHERE id = ?1 AND archived = 0", params![id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?, row.get::<_, String>(2)?))).map_err(|error| format!("cannot find active account '{id}': {error}"))?;
    let balance = prompt_amount_default(
        &format!("New balance [£{}]: ", format_currency("", current.1)),
        current.1,
    )?;
    if balance < 0.0 {
        return Err("balance cannot be negative".to_string());
    }
    let subcategory = prompt_default(&format!("Subcategory [{}]: ", current.2), &current.2)?;
    connection.execute("UPDATE tracker_accounts SET balance_gbp = ?1, subcategory = ?2, updated_at = ?3 WHERE id = ?4", params![balance, subcategory, today(), id]).map_err(|error| format!("cannot update account: {error}"))?;
    connection.execute("INSERT INTO tracker_valuations (account_id, valuation_date, balance_gbp, source) VALUES (?1, ?2, ?3, 'manual')", params![id, today(), balance]).map_err(|error| format!("cannot save account history: {error}"))?;
    println!(
        "Updated {} to {}.",
        current.0,
        format_currency("£", balance)
    );
    Ok(())
}

fn choose_account_id(connection: &Connection) -> Result<String, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, name, balance_gbp FROM tracker_accounts WHERE archived = 0 ORDER BY name",
        )
        .map_err(|error| format!("cannot query accounts: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(|error| format!("cannot list accounts: {error}"))?;
    let mut accounts = Vec::new();
    println!("Select an account to update:");
    for (number, row) in rows.enumerate() {
        let account = row.map_err(|error| error.to_string())?;
        println!(
            "  {:>3}. {:<30} {:>15}",
            number + 1,
            account.1,
            format_currency("£", account.2)
        );
        accounts.push(account);
    }
    if accounts.is_empty() {
        return Err("no active accounts found".to_string());
    }
    let choice = required_prompt("Choice: ")?;
    let index = choice
        .parse::<usize>()
        .map_err(|_| "enter an account number".to_string())?;
    accounts
        .get(index.saturating_sub(1))
        .map(|account| account.0.clone())
        .ok_or_else(|| "account choice is out of range".to_string())
}

fn archive_account(arguments: &[String]) -> Result<(), String> {
    let (_, connection) = tracker_connection()?;
    let id = arguments
        .first()
        .ok_or_else(|| "usage: cargo run -- archive <account-id> [--restore]".to_string())?;
    let archived = !arguments.iter().any(|argument| argument == "--restore");
    connection
        .execute(
            "UPDATE tracker_accounts SET archived = ?1 WHERE id = ?2",
            params![archived, id],
        )
        .map_err(|error| format!("cannot update account archive state: {error}"))?;
    println!(
        "{} account '{id}'.",
        if archived { "Archived" } else { "Restored" }
    );
    Ok(())
}

fn manage_subcategories(arguments: &[String]) -> Result<(), String> {
    let (_, connection) = tracker_connection()?;
    let action = arguments.first().map(String::as_str).unwrap_or("list");
    match action {
        "list" => {
            let mut statement = connection
                .prepare("SELECT id, name, archived FROM subcategories ORDER BY name")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (id, name, archived) = row.map_err(|error| error.to_string())?;
                println!(
                    "{id:<5} {name}{}",
                    if archived { " [archived]" } else { "" }
                );
            }
        }
        "add" => {
            let name = if let Some(name) = arguments.get(1) {
                name.clone()
            } else {
                required_prompt("Subcategory name: ")?
            };
            connection
                .execute(
                    "INSERT INTO subcategories (name) VALUES (?1)",
                    params![name],
                )
                .map_err(|error| format!("cannot add subcategory: {error}"))?;
            println!("Added subcategory '{name}'.");
        }
        "delete" => {
            let name = arguments
                .get(1)
                .ok_or_else(|| "usage: subcategory delete <name>".to_string())?;
            connection
                .execute(
                    "UPDATE subcategories SET archived = 1 WHERE name = ?1",
                    params![name],
                )
                .map_err(|error| format!("cannot archive subcategory: {error}"))?;
            println!("Archived subcategory '{name}'.");
        }
        "rename" => {
            let old = arguments
                .get(1)
                .ok_or_else(|| "usage: subcategory rename <old-name> <new-name>".to_string())?;
            let new = arguments
                .get(2)
                .ok_or_else(|| "usage: subcategory rename <old-name> <new-name>".to_string())?;
            connection
                .execute(
                    "UPDATE subcategories SET name = ?1 WHERE name = ?2",
                    params![new, old],
                )
                .map_err(|error| format!("cannot rename subcategory: {error}"))?;
            println!("Renamed subcategory '{old}' to '{new}'.");
        }
        _ => return Err("usage: subcategory [list|add|delete|rename]".to_string()),
    }
    Ok(())
}

fn total_transactions() -> Result<(), String> {
    let (config, connection) = tracker_connection()?;
    let (assets, liabilities) = account_totals(&connection)?;
    println!("{}", config.name);
    println!("Assets:      {}", format_currency("£", assets));
    println!("Liabilities: {}", format_currency("£", liabilities));
    println!(
        "Net balance: {}",
        format_currency("£", assets - liabilities)
    );
    Ok(())
}

fn account_totals(connection: &Connection) -> Result<(f64, f64), String> {
    connection
        .query_row("SELECT COALESCE(SUM(CASE WHEN kind = 'liability' THEN 0 ELSE balance_gbp * ownership_percent / 100 END), 0), COALESCE(SUM(CASE WHEN kind = 'liability' THEN balance_gbp * ownership_percent / 100 ELSE 0 END), 0) FROM tracker_accounts WHERE archived = 0", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| format!("cannot calculate totals: {error}"))
}

fn unique_account_id(connection: &Connection, name: &str) -> Result<String, String> {
    let base = slugify(name);
    let mut id = base.clone();
    let mut suffix = 2;
    while connection
        .query_row(
            "SELECT COUNT(*) FROM tracker_accounts WHERE id = ?1",
            params![id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("cannot check account ID: {error}"))?
        > 0
    {
        id = format!("{base}_{suffix}");
        suffix += 1;
    }
    Ok(id)
}

fn ensure_unique_account_name(connection: &Connection, name: &str) -> Result<(), String> {
    let existing = connection
        .query_row(
            "SELECT id, name, archived FROM tracker_accounts WHERE lower(trim(name)) = lower(trim(?1)) LIMIT 1",
            params![name],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?)),
        )
        .optional()
        .map_err(|error| format!("cannot check for duplicate account: {error}"))?;
    if let Some((id, existing_name, archived)) = existing {
        let status = if archived { "archived" } else { "active" };
        return Err(format!(
            "account '{existing_name}' already exists ({status}, id: {id})"
        ));
    }
    Ok(())
}

fn required_prompt(message: &str) -> Result<String, String> {
    let value = prompt(message)?;
    if value.is_empty() {
        Err("value cannot be empty".to_string())
    } else {
        Ok(value)
    }
}

fn prompt_amount_default(message: &str, default: f64) -> Result<f64, String> {
    let value = prompt(message)?;
    if value.is_empty() {
        Ok(default)
    } else {
        value
            .replace(',', "")
            .parse::<f64>()
            .map_err(|_| "invalid amount".to_string())
    }
}

fn import_spreadsheet(arguments: &[String]) -> Result<(), String> {
    println!("Import historical account data");
    if arguments.len() > 1 {
        return Err("usage: cargo run -- import [spreadsheet-path]".to_string());
    }
    let path = if let Some(path) = arguments.first() {
        path.clone()
    } else {
        prompt_default(
            &format!("Spreadsheet path [{DEFAULT_SPREADSHEET_PATH}]: "),
            DEFAULT_SPREADSHEET_PATH,
        )?
    };
    if !std::path::Path::new(&path).exists() {
        return Err(format!("spreadsheet not found: {path}"));
    }

    println!("\nSelect a database:");
    let tracker_path = tracker_storage_path().unwrap_or_else(|| "data/tracker.sqlite3".to_string());
    println!("  1. Active tracker — {tracker_path} (persistent, recommended)");
    println!("  2. Investment history — {SQLITE_PATH}");
    println!("  3. In-memory SQLite (temporary)");
    let choice = prompt("Choice [1]: ")?;
    let choice = if choice.is_empty() {
        1
    } else {
        choice
            .parse::<u8>()
            .map_err(|_| "please choose 1, 2, or 3".to_string())?
    };
    let mut connection = match choice {
        1 => {
            if let Some(parent) = std::path::Path::new(&tracker_path).parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create database directory: {error}"))?;
            }
            Connection::open(&tracker_path)
                .map_err(|error| format!("cannot open SQLite database: {error}"))?
        }
        2 => Connection::open(SQLITE_PATH)
            .map_err(|error| format!("cannot open SQLite database: {error}"))?,
        3 => Connection::open_in_memory()
            .map_err(|error| format!("cannot open in-memory SQLite database: {error}"))?,
        _ => return Err("please choose 1, 2, or 3".to_string()),
    };

    create_tracker_schema(&connection)?;
    if choice == 2 {
        create_schema(&connection)?;
    }
    let imported = read_workbook(&path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("cannot start database transaction: {error}"))?;
    for account in &imported {
        transaction.execute("INSERT OR REPLACE INTO tracker_accounts (id, name, kind, balance_gbp, ownership_percent, subcategory, archived, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", params![account.id, account.name, account.kind, account.balance_gbp, account.ownership_percent, account.tag, account.archived, today()]).map_err(|error| format!("cannot save tracker account {}: {error}", account.name))?;
        transaction
            .execute(
                "DELETE FROM tracker_valuations WHERE account_id = ?1",
                params![account.id],
            )
            .map_err(|error| {
                format!(
                    "cannot replace tracker history for {}: {error}",
                    account.name
                )
            })?;
        if choice == 2 {
            transaction
                .execute(
                    "DELETE FROM valuations WHERE account_id = ?1",
                    params![account.id],
                )
                .map_err(|error| {
                    format!(
                        "cannot replace investment history for {}: {error}",
                        account.name
                    )
                })?;
            transaction
                .execute("DELETE FROM accounts WHERE id = ?1", params![account.id])
                .map_err(|error| {
                    format!(
                        "cannot replace investment account {}: {error}",
                        account.name
                    )
                })?;
            transaction
                .execute("INSERT INTO accounts (id, name, kind, currency, balance_gbp, ownership_percent, archived, tag, note) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)", params![account.id, account.name, account.kind, account.currency, account.balance_gbp, account.ownership_percent, account.archived, account.tag, account.note])
                .map_err(|error| format!("cannot save investment account {}: {error}", account.name))?;
        }
        for valuation in &account.valuations {
            transaction.execute("INSERT INTO tracker_valuations (account_id, valuation_date, balance_gbp, source) VALUES (?1, ?2, ?3, ?4)", params![account.id, valuation.date, valuation.balance_gbp, valuation.source]).map_err(|error| format!("cannot save valuation for {}: {error}", account.name))?;
            if choice == 2 {
                transaction.execute("INSERT INTO valuations (account_id, valuation_date, balance_gbp, source) VALUES (?1, ?2, ?3, ?4)", params![account.id, valuation.date, valuation.balance_gbp, valuation.source]).map_err(|error| format!("cannot save investment valuation for {}: {error}", account.name))?;
            }
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("cannot commit import: {error}"))?;
    let database_name = match choice {
        1 => tracker_path.as_str(),
        2 => SQLITE_PATH,
        _ => "in-memory SQLite",
    };
    let valuation_count: usize = imported
        .iter()
        .map(|account| account.valuations.len())
        .sum();
    println!(
        "Imported {} accounts and {} historical valuations into {database_name}.",
        imported.len(),
        valuation_count
    );
    Ok(())
}

struct ImportedAccount {
    id: String,
    name: String,
    kind: String,
    currency: String,
    balance_gbp: f64,
    ownership_percent: f64,
    archived: bool,
    tag: String,
    note: String,
    valuations: Vec<ImportedValuation>,
}

struct ImportedValuation {
    date: String,
    balance_gbp: f64,
    source: String,
}

fn create_schema(connection: &Connection) -> Result<(), String> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS accounts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            currency TEXT NOT NULL,
            balance_gbp REAL NOT NULL,
            ownership_percent REAL NOT NULL,
            archived INTEGER NOT NULL,
            tag TEXT NOT NULL,
            note TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS valuations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT NOT NULL REFERENCES accounts(id),
            valuation_date TEXT NOT NULL,
            balance_gbp REAL NOT NULL,
            source TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_valuations_account_date ON valuations(account_id, valuation_date);")
        .map_err(|error| format!("cannot create database schema: {error}"))
}

fn read_workbook(path: &str) -> Result<Vec<ImportedAccount>, String> {
    let mut workbook =
        open_workbook_auto(path).map_err(|error| format!("cannot open spreadsheet: {error}"))?;
    let mut accounts = Vec::new();
    for sheet_name in workbook.sheet_names().to_owned() {
        if sheet_name == "Overview" {
            continue;
        }
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|error| format!("cannot read sheet {sheet_name}: {error}"))?;
        let mut fields = HashMap::new();
        for row in range.rows() {
            if row.len() >= 2 {
                fields.insert(row[0].to_string(), row[1].to_string());
            }
        }
        let name = fields
            .get("name")
            .cloned()
            .unwrap_or_else(|| sheet_name.clone());
        let id = fields.get("ID").cloned().unwrap_or_else(|| slugify(&name));
        let currency = fields
            .get("currency")
            .cloned()
            .unwrap_or_else(|| "GBP".to_string());
        let tag = fields.get("tag").cloned().unwrap_or_default();
        let kind = if tag.contains("Debt") {
            "liability"
        } else if tag.contains("Savings") {
            "savings"
        } else if tag.contains("Crypto") {
            "investment"
        } else if name.to_ascii_lowercase().contains("pension") {
            "pension"
        } else {
            "investment"
        };
        let ownership_percent = fields
            .get("ownership_ratio")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0)
            * 100.0;
        let archived = fields
            .get("is_archived")
            .map(|value| value == "true")
            .unwrap_or(false);
        let note = fields.get("note").cloned().unwrap_or_default();
        let mut valuations = parse_valuations(fields.get("values"), "values")?;
        valuations.extend(parse_valuations(
            fields.get("values_market"),
            "values_market",
        )?);
        valuations.sort_by(|left, right| left.date.cmp(&right.date));
        let balance_gbp = valuations
            .last()
            .map(|valuation| valuation.balance_gbp)
            .unwrap_or(0.0);
        accounts.push(ImportedAccount {
            id,
            name,
            kind: kind.to_string(),
            currency,
            balance_gbp,
            ownership_percent,
            archived,
            tag,
            note,
            valuations,
        });
    }
    Ok(accounts)
}

fn parse_valuations(
    value: Option<&String>,
    source: &str,
) -> Result<Vec<ImportedValuation>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let content = value.trim().trim_start_matches('{').trim_end_matches('}');
    if content.is_empty() {
        return Ok(Vec::new());
    }
    content
        .split(',')
        .map(|entry| {
            let (timestamp, amount) = entry
                .trim()
                .split_once(':')
                .ok_or_else(|| format!("invalid valuation entry: {entry}"))?;
            let timestamp = timestamp
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("invalid valuation timestamp: {timestamp}"))?;
            let balance_gbp = amount
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("invalid valuation amount: {amount}"))?;
            let (year, month, day) = civil_date_from_days(timestamp / 86_400_000);
            Ok(ImportedValuation {
                date: format!("{year:04}-{month:02}-{day:02}"),
                balance_gbp,
                source: source.to_string(),
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct ExchangeRateResponse {
    rates: HashMap<String, f64>,
}

fn fetch_gbp_usd_rate() -> Result<f64, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|error| format!("could not create rates client: {error}"))?
        .get("https://api.frankfurter.dev/v1/latest?base=GBP&symbols=USD")
        .send()
        .map_err(|error| format!("rates request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("rates service returned an error: {error}"))?
        .json::<ExchangeRateResponse>()
        .map_err(|error| format!("invalid rates response: {error}"))?;
    response
        .rates
        .get("USD")
        .copied()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .ok_or_else(|| "USD rate was missing from the rates response".to_string())
}

fn format_currency(symbol: &str, amount: f64) -> String {
    let rounded = amount.round() as i64;
    let sign = if rounded < 0 { "-" } else { "" };
    let digits = rounded.unsigned_abs().to_string();
    let first_group_length = digits.len() % 3;
    let mut grouped = String::new();

    if first_group_length > 0 {
        grouped.push_str(&digits[..first_group_length]);
    }
    for chunk in digits.as_bytes()[first_group_length..].chunks(3) {
        if !grouped.is_empty() {
            grouped.push(',');
        }
        grouped.push_str(std::str::from_utf8(chunk).expect("digits are valid UTF-8"));
    }
    format!("{symbol}{sign}{grouped}")
}

fn prompt(message: &str) -> Result<String, String> {
    print!("{message}");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|error| error.to_string())?;
    Ok(value.trim().to_string())
}

fn prompt_default(message: &str, default: &str) -> Result<String, String> {
    let value = prompt(message)?;
    Ok(if value.is_empty() {
        default.to_string()
    } else {
        value
    })
}

fn prompt_amount(message: &str) -> Result<f64, String> {
    let value = prompt(message)?;
    if value.is_empty() {
        return Ok(0.0);
    }
    value
        .replace(',', "")
        .parse::<f64>()
        .map_err(|_| format!("'{value}' is not a valid amount"))
}

fn slugify(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn print_help() {
    println!(
        "Usage:\n  cargo run -- init <name>                         Initialize a tracker\n  cargo run -- add                                Add an account\n  cargo run -- list [--archived] [--ids]           List account balances and totals\n  cargo run -- update [account-id]                Update an account balance\n  cargo run -- archive <account-id> [--restore]   Archive or restore an account\n  cargo run -- subcategory [list|add|delete|rename] Manage subcategories\n  cargo run -- total                               Show assets, liabilities, and net balance\n  cargo run -- import [spreadsheet-path]          Import account history"
    );
}

fn today() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_secs() as i64
        / 86_400;
    let (year, month, day) = civil_date_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

// Converts Unix-epoch days to a Gregorian calendar date.
fn civil_date_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}
