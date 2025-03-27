use futures::future::join_all;
use reqwest::{Client, Proxy, redirect::Policy};
use serde::Deserialize;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncBufReadExt;

#[derive(Debug, Deserialize)]
struct Response {
    data: Vec<Data>,
}

#[derive(Debug, Deserialize)]
struct Data {
    amount: f64,
}

async fn get_airdrop_amount_with_retry(
    address: &str,
    max_retries: u32,
    proxy: Option<String>,
) -> Result<f64, reqwest::Error> {
    let mut retries = 0;
    loop {
        match get_airdrop_amount(address, proxy.clone()).await {
            Ok(amount) => return Ok(amount),
            Err(e) if retries < max_retries => {
                eprintln!(
                    "Retry {}/{} for address {}: {}",
                    retries + 1,
                    max_retries,
                    address,
                    e
                );
                retries += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn get_airdrop_amount(address: &str, proxy: Option<String>) -> Result<f64, reqwest::Error> {
    let mut client_builder = Client::builder().redirect(Policy::none());

    if let Some(proxy_url) = proxy {
        client_builder = client_builder.proxy(Proxy::all(format!("http://{}", proxy_url))?);
    }

    let client = client_builder.build()?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();

    let url = format!(
        "https://opapi.kiloex.io/point/queryKiloAccountAwardFlow?type=0&account={}&t={}",
        address, timestamp
    );
    let res = client.get(url).send().await?.json::<Response>().await?;

    if res.data.is_empty() {
        return Ok(0.0);
    }

    let total_amount: f64 = res.data.iter().map(|d| d.amount).sum();

    Ok(total_amount)
}

pub async fn read_lines(path: impl AsRef<Path>) -> Result<Vec<String>, std::io::Error> {
    let file = tokio::fs::File::open(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut lines = reader.lines();

    let mut contents = vec![];
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            contents.push(trimmed.to_string());
        }
    }

    Ok(contents)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    const ADDRESSES_PATH_KEY: &str = "data/addresses.txt";
    const PROXIES_PATH_KEY: &str = "data/proxies.txt";
    const MAX_RETRIES: u32 = 10;

    let addresses = match read_lines(ADDRESSES_PATH_KEY).await {
        Ok(addresses) => addresses,
        Err(e) => {
            eprintln!("Error reading addresses file: {}", e);
            return Ok(());
        }
    };

    let proxies = match read_lines(PROXIES_PATH_KEY).await {
        Ok(proxies) if !proxies.is_empty() => proxies,
        _ => Vec::new(),
    };

    let mut proxy_iter = proxies.into_iter().cycle();

    let futures = addresses.iter().map(|address| {
        let proxy = proxy_iter.next();

        async move {
            match get_airdrop_amount_with_retry(address, MAX_RETRIES, proxy.clone()).await {
                Ok(amount) => {
                    println!("Address {}: {} KILO", address, amount);
                    Some(amount)
                }
                Err(e) => {
                    eprintln!("Failed after retries for address {}: {}", address, e);
                    None
                }
            }
        }
    });

    let results = join_all(futures).await;
    let total_sum: f64 = results.into_iter().filter_map(|x| x).sum();
    println!("Total sum across all addresses: {} KILO", total_sum);

    Ok(())
}
