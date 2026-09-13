# Repository Guidelines

## Project Structure & Module Organization

This repository is a Rust CLI with tracker data stored in SQLite. New contributors should use the conventional Cargo layout:

- `Cargo.toml` — package metadata and dependencies.
- `src/main.rs` for a binary, or `src/lib.rs` for a library.
- `src/` submodules grouped by domain (for example, `src/portfolio/` and `src/market_data/`).
- `tests/` for integration tests and `benches/` for benchmarks when needed.
- `README.md` for setup and usage; keep sample data or fixtures under `tests/fixtures/`.

Keep modules focused and expose only the interfaces required by their callers.

## Build, Test, and Development Commands

After the Cargo project is initialized, use:

- `cargo fmt --all -- --check` — verify formatting.
- `cargo clippy --all-targets --all-features -- -D warnings` — run lint checks with warnings treated as errors.
- `cargo test --all-targets` — run unit and integration tests.
- `cargo build` — compile the project locally.
- `cargo run -- init <name>` — initialize a named tracker and create its storage file.
- `cargo run -- add` — interactively add an account and its current balance; duplicate names are rejected case-insensitively.
- `cargo run -- list` — list active account balances and show GBP/USD assets, liabilities, and net balance; use `--archived` to include archived accounts or `--ids` to show account IDs.
- `cargo run -- update [account-id]` — modify an existing account balance.
- `cargo run -- archive <account-id>` — archive an account; use `--restore` to recover it.
- `cargo run -- subcategory list|add|delete|rename` — manage custom subcategories.
- `cargo run -- total` — calculate assets, liabilities, and net balance.
- `cargo run -- import [spreadsheet-path]` — import workbook account history into the selected SQLite database without duplicating existing rows.

Run formatting and tests before opening a pull request. Add any required environment variables or external services to `README.md`; do not commit credentials.

All tracker account data is kept in the SQLite file created by `init`, with the active tracker selected by `data/tracker.json`. Archived accounts are excluded from totals. Spreadsheet imports and manual account updates use the selected tracker database. `list` obtains the current GBP/USD rate at runtime and reports USD as unavailable if the network lookup fails. There is no dashboard subcommand; account totals are part of `list` and `total`.

## Coding Style & Naming Conventions

Use standard `rustfmt` formatting and four-space indentation. Name modules and files in `snake_case`, types and traits in `PascalCase`, functions and variables in `snake_case`, and constants in `SCREAMING_SNAKE_CASE`. Prefer explicit domain types and error handling with `Result` over panics in production paths. Document public APIs and explain non-obvious financial assumptions near the relevant code.

## Testing Guidelines

Place unit tests beside the implementation in `#[cfg(test)]` modules and cross-module behavior in `tests/`. Name tests after the behavior they verify, such as `rejects_negative_position`. Include edge cases for money, dates, rounding, and missing market data. Keep tests deterministic and avoid live service calls.

## Commit & Pull Request Guidelines

No commit history exists yet, so use imperative, focused commit subjects (for example, `Add portfolio valuation model`). Pull requests should explain the change and its financial or behavioral impact, link related issues, describe validation commands run, and call out configuration or schema changes. Include example output or screenshots when the change affects user-facing reports.
