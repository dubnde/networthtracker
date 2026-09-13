use crate::{Error, Result};
use serde::Deserialize;
use std::collections::HashMap;

pub trait ExchangeRateProvider {
    fn gbp_to_usd(&self) -> Result<f64>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HttpExchangeRateProvider;

impl ExchangeRateProvider for HttpExchangeRateProvider {
    fn gbp_to_usd(&self) -> Result<f64> {
        #[derive(Deserialize)]
        struct Response {
            rates: HashMap<String, f64>,
        }
        let response = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| Error::External(e.to_string()))?
            .get("https://api.frankfurter.dev/v1/latest?base=GBP&symbols=USD")
            .send()
            .map_err(|e| Error::External(e.to_string()))?
            .error_for_status()
            .map_err(|e| Error::External(e.to_string()))?
            .json::<Response>()
            .map_err(|e| Error::External(e.to_string()))?;
        response
            .rates
            .get("USD")
            .copied()
            .filter(|rate| rate.is_finite() && *rate > 0.0)
            .ok_or_else(|| Error::External("USD rate was missing from the rates response".into()))
    }
}
