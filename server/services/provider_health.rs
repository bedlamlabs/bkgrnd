use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::Client;
use tokio::{sync::Mutex, time::timeout};
use url::{Host, Url};

const HEALTH_CACHE_TTL: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct ProviderHealth {
    http: Client,
    ping_url: Option<Url>,
    cache: Arc<Mutex<Option<(Instant, bool)>>>,
}

impl ProviderHealth {
    pub fn new(http: Client, base_url: Option<&str>) -> Result<Self, String> {
        let ping_url = base_url.map(validate_ping_url).transpose()?;
        Ok(Self {
            http,
            ping_url,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn available(&self) -> bool {
        let Some(ping_url) = self.ping_url.as_ref() else {
            return true;
        };
        let mut cache = self.cache.lock().await;
        if let Some((checked_at, available)) = *cache {
            if checked_at.elapsed() < HEALTH_CACHE_TTL {
                return available;
            }
        }
        let available = matches!(
            timeout(
                Duration::from_secs(2),
                self.http.get(ping_url.clone()).send()
            )
            .await,
            Ok(Ok(response)) if response.status().is_success()
        );
        *cache = Some((Instant::now(), available));
        available
    }
}

fn validate_ping_url(base_url: &str) -> Result<Url, String> {
    let mut parsed =
        Url::parse(base_url).map_err(|error| format!("invalid WOPR_POT_PROVIDER_URL: {error}"))?;
    let loopback = match parsed.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    if parsed.scheme() != "http"
        || !loopback
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !matches!(parsed.path(), "" | "/")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("WOPR_POT_PROVIDER_URL must be a credential-free loopback HTTP origin".into());
    }
    parsed.set_path("/ping");
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_health_accepts_only_loopback_http_origins() {
        for value in [
            "http://127.0.0.1:4416",
            "http://localhost:4416/",
            "http://[::1]:4416",
        ] {
            assert!(validate_ping_url(value).is_ok(), "{value}");
        }
        for value in [
            "https://127.0.0.1:4416",
            "http://example.com:4416",
            "http://user:pass@127.0.0.1:4416",
            "http://127.0.0.1:4416/other",
            "http://127.0.0.1:4416?target=other",
        ] {
            assert!(validate_ping_url(value).is_err(), "{value}");
        }
    }
}
