use crate::{Error, Result, slugify};
use calamine::{Reader, open_workbook_auto};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedAccount {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub balance_gbp: f64,
    pub ownership_percent: f64,
    pub archived: bool,
    pub tag: String,
    pub valuations: Vec<ImportedValuation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedValuation {
    pub date: String,
    pub balance_gbp: f64,
    pub source: String,
}

pub struct SpreadsheetImporter;
impl SpreadsheetImporter {
    pub fn read(path: impl AsRef<std::path::Path>) -> Result<Vec<ImportedAccount>> {
        let mut workbook = open_workbook_auto(path).map_err(|e| Error::Import(e.to_string()))?;
        let mut accounts = Vec::new();
        for sheet in workbook.sheet_names().to_owned() {
            if sheet == "Overview" {
                continue;
            }
            let range = workbook
                .worksheet_range(&sheet)
                .map_err(|e| Error::Import(e.to_string()))?;
            let mut fields = HashMap::new();
            for row in range.rows() {
                if row.len() >= 2 {
                    fields.insert(row[0].to_string(), row[1].to_string());
                }
            }
            let name = fields.get("name").cloned().unwrap_or(sheet.clone());
            let id = fields.get("ID").cloned().unwrap_or_else(|| slugify(&name));
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
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(1.0)
                * 100.0;
            let archived = fields
                .get("is_archived")
                .map(|v| v == "true")
                .unwrap_or(false);
            let mut valuations = parse_valuations(fields.get("values"), "values")?;
            valuations.extend(parse_valuations(
                fields.get("values_market"),
                "values_market",
            )?);
            valuations.sort_by(|a, b| a.date.cmp(&b.date));
            let balance_gbp = valuations.last().map(|v| v.balance_gbp).unwrap_or(0.0);
            accounts.push(ImportedAccount {
                id,
                name,
                kind: kind.to_string(),
                balance_gbp,
                ownership_percent,
                archived,
                tag,
                valuations,
            });
        }
        Ok(accounts)
    }
}
fn parse_valuations(value: Option<&String>, source: &str) -> Result<Vec<ImportedValuation>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let content = value.trim().trim_start_matches('{').trim_end_matches('}');
    if content.is_empty() {
        return Ok(Vec::new());
    };
    content
        .split(',')
        .map(|entry| {
            let (timestamp, amount) = entry
                .trim()
                .split_once(':')
                .ok_or_else(|| Error::Import(format!("invalid valuation entry: {entry}")))?;
            let timestamp = timestamp
                .trim()
                .parse::<i64>()
                .map_err(|_| Error::Import(format!("invalid valuation timestamp: {timestamp}")))?;
            let balance_gbp = amount
                .trim()
                .parse::<f64>()
                .map_err(|_| Error::Import(format!("invalid valuation amount: {amount}")))?;
            let days = timestamp / 86_400_000;
            Ok(ImportedValuation {
                date: date_from_days(days),
                balance_gbp,
                source: source.into(),
            })
        })
        .collect()
}
fn date_from_days(days: i64) -> String {
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
