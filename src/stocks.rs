use crate::config::StockConfig;
use serde_json::Value;
use std::process::{Command, Output};

#[derive(Clone, Debug)]
pub struct StockQuote {
    pub symbol: String,
    pub price: f64,
    pub change_percent: Option<f64>,
}

impl StockQuote {
    pub fn label(&self) -> &str {
        match self.symbol.as_str() {
            "^NSEI" => "NIFTY 50",
            "^BSESN" => "SENSEX",
            _ => &self.symbol,
        }
    }
}

pub fn fetch(config: &StockConfig) -> (Vec<StockQuote>, Vec<String>) {
    if !config.enabled || config.symbols.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut quotes = Vec::new();
    let mut errors = Vec::new();
    for symbol in config.symbols.iter().take(config.max_items) {
        match fetch_one(symbol, &config.quote_url_template) {
            Ok(quote) => quotes.push(quote),
            // Keep provider details such as Yahoo's 429 response out of the
            // menu; a later automatic refresh retries the request.
            Err(_) => errors.push(format!("{symbol}: quote temporarily unavailable")),
        }
    }
    (quotes, errors)
}

fn fetch_one(symbol: &str, template: &str) -> Result<StockQuote, String> {
    match symbol {
        "^NSEI" => return fetch_nifty(),
        "^BSESN" => return fetch_sensex(),
        _ => {}
    }
    if !symbol
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '^'))
    {
        return Err("symbol may only contain letters, numbers, '.', '-', or '^'".into());
    }
    if !template.starts_with("https://") || !template.contains("{symbol}") {
        return Err("quote_url_template must be an HTTPS URL containing {symbol}".into());
    }

    let url = template.replace("{symbol}", symbol);
    let output = curl(&url, &[])?;
    parse_quote(symbol, &String::from_utf8_lossy(&output.stdout))
}

fn fetch_nifty() -> Result<StockQuote, String> {
    let output = curl(
        "https://www.nseindia.com/api/allIndices",
        &["-H", "Accept: application/json"],
    )?;
    let data: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid NSE response: {error}"))?;
    let index = data["data"]
        .as_array()
        .and_then(|indices| {
            indices
                .iter()
                .find(|index| index["indexSymbol"].as_str() == Some("NIFTY 50"))
        })
        .ok_or("NIFTY 50 was not present in NSE's response")?;
    Ok(StockQuote {
        symbol: "^NSEI".into(),
        price: index["last"]
            .as_f64()
            .ok_or("NSE response did not contain the NIFTY price")?,
        change_percent: index["percentChange"].as_f64(),
    })
}

fn fetch_sensex() -> Result<StockQuote, String> {
    let output = curl(
        "https://api.bseindia.com/RealTimeBseIndiaAPI/api/GetSensexData/w",
        &[
            "-H",
            "Accept: application/json, text/plain, */*",
            "-H",
            "Referer: https://www.bseindia.com/",
            "-H",
            "Origin: https://www.bseindia.com",
        ],
    )?;
    let data: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid BSE response: {error}"))?;
    let index = data
        .as_array()
        .and_then(|indices| indices.first())
        .ok_or("SENSEX was not present in BSE's response")?;
    let price = index["ltp"]
        .as_str()
        .map(|value| value.replace(',', ""))
        .and_then(|value| value.parse::<f64>().ok())
        .ok_or("BSE response did not contain the SENSEX price")?;
    let change_percent = index["perchg"]
        .as_str()
        .and_then(|value| value.replace(',', "").parse::<f64>().ok());
    Ok(StockQuote {
        symbol: "^BSESN".into(),
        price,
        change_percent,
    })
}

fn curl(url: &str, headers: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("curl");
    command.args([
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "--max-time",
        "10",
        "--compressed",
        "--user-agent",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36",
    ]);
    command.args(headers).arg(url);
    let output = command
        .output()
        .map_err(|error| format!("could not start curl: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(output)
}

fn parse_quote(symbol: &str, body: &str) -> Result<StockQuote, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("invalid quote response: {error}"))?;
    let meta = &value["chart"]["result"][0]["meta"];
    let price = meta["regularMarketPrice"]
        .as_f64()
        .ok_or("quote response did not contain a regular market price")?;
    let previous = meta["chartPreviousClose"]
        .as_f64()
        .or_else(|| meta["previousClose"].as_f64());
    let change_percent = previous
        .filter(|previous| *previous != 0.0)
        .map(|previous| (price - previous) / previous * 100.0);

    Ok(StockQuote {
        symbol: symbol.to_ascii_uppercase(),
        price,
        change_percent,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_quote, StockQuote};

    #[test]
    fn parses_a_chart_quote() {
        let quote = parse_quote(
            "AAPL",
            r#"{"chart":{"result":[{"meta":{"regularMarketPrice":210.0,"chartPreviousClose":200.0}}]}}"#,
        )
        .unwrap();
        assert_eq!(quote.symbol, "AAPL");
        assert_eq!(quote.price, 210.0);
        assert_eq!(quote.change_percent, Some(5.0));
    }

    #[test]
    fn labels_default_indian_indices() {
        assert_eq!(
            StockQuote {
                symbol: "^NSEI".into(),
                price: 0.0,
                change_percent: None
            }
            .label(),
            "NIFTY 50"
        );
        assert_eq!(
            StockQuote {
                symbol: "^BSESN".into(),
                price: 0.0,
                change_percent: None
            }
            .label(),
            "SENSEX"
        );
    }
}
