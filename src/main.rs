use investments::{
    AccountQuery, CreateAccount, Error, ExchangeRateProvider, HttpExchangeRateProvider, Result,
    SpreadsheetImporter, TrackerService, UpdateAccount,
};
use std::io::{self, Write};
use std::path::Path;

const DEFAULT_SPREADSHEET_PATH: &str = "/Users/dchingalande/Library/Mobile Documents/com~apple~CloudDocs/Downloads/Worth_it_export.xlsx";

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
fn run(arguments: Vec<String>) -> Result<()> {
    let command = arguments.first().map(String::as_str).unwrap_or("help");
    if matches!(command, "help" | "--help" | "-h") {
        print_help();
        return Ok(());
    }
    if command == "init" {
        return init(&arguments[1..]);
    }
    let service = active_service()?;
    match command {
        "add" => add(&service),
        "list" => list(&service, &arguments[1..]),
        "update" => update(&service, &arguments[1..]),
        "archive" => archive(&service, &arguments[1..]),
        "subcategory" => subcategory(&service, &arguments[1..]),
        "total" => total(&service),
        "import" => import(&service, &arguments[1..]),
        other => Err(Error::Validation(format!("unknown command '{other}'"))),
    }
}
fn init(arguments: &[String]) -> Result<()> {
    if arguments.len() != 1 || arguments[0].trim().is_empty() {
        return Err(Error::Validation(
            "usage: cargo run -- init <tracker-name>".into(),
        ));
    }
    let name = arguments[0].trim();
    let path = format!("data/{}.sqlite3", investments::slugify(name));
    if Path::new("data/tracker.json").exists() {
        return Err(Error::Validation("a tracker is already initialized".into()));
    }
    std::fs::create_dir_all("data")?;
    TrackerService::open(&path, name)?;
    std::fs::write(
        "data/tracker.json",
        format!(
            "{{\n  \"name\": {:?},\n  \"storage_path\": {:?}\n}}\n",
            name, path
        ),
    )?;
    println!("Initialized tracker '{name}' in {path}.");
    Ok(())
}
fn active_service() -> Result<TrackerService> {
    let text = std::fs::read_to_string("data/tracker.json").map_err(|_| {
        Error::Validation("tracker is not initialized; run `cargo run -- init <name>` first".into())
    })?;
    let config: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::Validation(format!("invalid tracker configuration: {e}")))?;
    let name = config["name"].as_str().unwrap_or("tracker");
    let path = config["storage_path"]
        .as_str()
        .ok_or_else(|| Error::Validation("tracker configuration has no storage path".into()))?;
    TrackerService::open(path, name)
}
fn add(service: &TrackerService) -> Result<()> {
    let request = CreateAccount {
        name: required("Account name: ")?,
        kind: prompt_default("Account type [investment]: ", "investment")?,
        balance_gbp: amount("Current balance [£]: ")?,
        ownership_percent: 100.0,
        subcategory: prompt_default("Subcategory [general]: ", "general")?,
    };
    let account = service.create_account(request)?;
    println!(
        "Added account '{}' with £{}.",
        account.name,
        format_amount(account.balance_gbp)
    );
    Ok(())
}
fn list(service: &TrackerService, args: &[String]) -> Result<()> {
    let accounts = service.accounts(AccountQuery {
        include_archived: args.iter().any(|a| a == "--archived"),
    })?;
    println!(
        "{:<32} {:>15}  {:<14} Subcategory",
        "Account", "Balance", "Type"
    );
    println!("{}", "-".repeat(98));
    for account in accounts {
        println!(
            "{:<32} {:>15}  {:<14} {}{}",
            account.name,
            format_currency("£", account.balance_gbp),
            account.kind,
            account.subcategory,
            if account.archived { " [archived]" } else { "" }
        );
    }
    Ok(())
}
fn update(service: &TrackerService, args: &[String]) -> Result<()> {
    let id = args
        .first()
        .cloned()
        .ok_or_else(|| Error::Validation("usage: update <account-id>".into()))?;
    let account = service.account(&id)?;
    let balance = prompt_amount_default(
        &format!("New balance [£{}]: ", format_amount(account.balance_gbp)),
        account.balance_gbp,
    )?;
    let subcategory = prompt_default(
        &format!("Subcategory [{}]: ", account.subcategory),
        &account.subcategory,
    )?;
    let account = service.update_account(
        &id,
        UpdateAccount {
            balance_gbp: balance,
            subcategory,
        },
    )?;
    println!(
        "Updated {} to £{}.",
        account.name,
        format_amount(account.balance_gbp)
    );
    Ok(())
}
fn archive(service: &TrackerService, args: &[String]) -> Result<()> {
    let id = args
        .first()
        .ok_or_else(|| Error::Validation("usage: archive <account-id> [--restore]".into()))?;
    let restored = args.iter().any(|a| a == "--restore");
    service.set_archived(id, !restored)?;
    println!(
        "{} account '{id}'.",
        if restored { "Restored" } else { "Archived" }
    );
    Ok(())
}
fn subcategory(service: &TrackerService, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            for item in service.subcategories(true)? {
                println!(
                    "{} {}{}",
                    item.id,
                    item.name,
                    if item.archived { " [archived]" } else { "" }
                );
            }
            Ok(())
        }
        "add" => {
            let name = args
                .get(1)
                .cloned()
                .unwrap_or(required("Subcategory name: ")?);
            service.add_subcategory(&name)?;
            Ok(())
        }
        "delete" => service.archive_subcategory(
            args.get(1)
                .ok_or_else(|| Error::Validation("usage: subcategory delete <name>".into()))?,
        ),
        "rename" => service.rename_subcategory(
            args.get(1)
                .ok_or_else(|| Error::Validation("missing old name".into()))?,
            args.get(2)
                .ok_or_else(|| Error::Validation("missing new name".into()))?,
        ),
        _ => Err(Error::Validation(
            "usage: subcategory [list|add|delete|rename]".into(),
        )),
    }
}
fn total(service: &TrackerService) -> Result<()> {
    let totals = service.totals()?;
    let rate = HttpExchangeRateProvider.gbp_to_usd().ok();
    println!("{}", service.tracker().name);
    print_totals_table(&totals, rate);
    Ok(())
}

fn print_totals_table(totals: &investments::Totals, rate: Option<f64>) {
    println!("\n{:<18} {:>15} {:>15}", "Type", "GBP", "USD");
    println!("{}", "-".repeat(52));
    for (label, amount) in [
        ("Assets", totals.assets_gbp),
        ("Liabilities", totals.liabilities_gbp),
        ("Networth", totals.net_gbp),
    ] {
        let usd = rate
            .map(|rate| format_currency("$", amount * rate))
            .unwrap_or_else(|| "unavailable".to_string());
        println!(
            "{:<18} {:>15} {:>15}",
            label,
            format_currency("£", amount),
            usd
        );
    }
}
fn import(service: &TrackerService, args: &[String]) -> Result<()> {
    let path = args.first().cloned().unwrap_or(prompt_default(
        &format!("Spreadsheet path [{DEFAULT_SPREADSHEET_PATH}]: "),
        DEFAULT_SPREADSHEET_PATH,
    )?);
    let accounts = SpreadsheetImporter::read(path)?;
    let count = service.import_accounts(&accounts)?;
    println!("Imported {count} accounts.");
    Ok(())
}
fn required(message: &str) -> Result<String> {
    let value = prompt(message)?;
    if value.is_empty() {
        Err(Error::Validation("value cannot be empty".into()))
    } else {
        Ok(value)
    }
}
fn prompt(message: &str) -> Result<String> {
    print!("{message}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_string())
}
fn prompt_default(message: &str, default: &str) -> Result<String> {
    let value = prompt(message)?;
    Ok(if value.is_empty() {
        default.into()
    } else {
        value
    })
}
fn prompt_amount_default(message: &str, default: f64) -> Result<f64> {
    let value = prompt(message)?;
    if value.is_empty() {
        Ok(default)
    } else {
        value
            .replace(',', "")
            .parse()
            .map_err(|_| Error::Validation("invalid amount".into()))
    }
}
fn amount(message: &str) -> Result<f64> {
    prompt_amount_default(message, 0.0)
}
fn format_amount(value: f64) -> String {
    format_currency("", value)
}

fn format_currency(symbol: &str, value: f64) -> String {
    let rounded = value.round() as i64;
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
fn print_help() {
    println!(
        "Usage:\n  cargo run -- init <name>\n  cargo run -- add\n  cargo run -- list [--archived]\n  cargo run -- update <account-id>\n  cargo run -- archive <account-id> [--restore]\n  cargo run -- subcategory [list|add|delete|rename]\n  cargo run -- total\n  cargo run -- import [spreadsheet-path]"
    );
}
