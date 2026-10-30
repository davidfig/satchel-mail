//! Photon, OpenStreetMap's search-as-you-type geocoder. Its public instance
//! asks for fair use, so the engine caches answers and the UI waits for a
//! pause in typing.

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use satchel_api::{PlaceSource, PlaceSuggestion};
use satchel_core::provider::PlaceSearch;
use serde_json::Value;

pub const PHOTON_URL: &str = "https://photon.komoot.io/api/";

pub struct Photon {
    url: String,
    http: OnceLock<reqwest::Client>,
}

impl Photon {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), http: OnceLock::new() }
    }

    fn http(&self) -> Result<&reqwest::Client> {
        if let Some(c) = self.http.get() {
            return Ok(c);
        }
        // OSM services ask clients to identify themselves.
        let c = crate::net::http_builder(false)
            .user_agent(concat!("SatchelMail/", env!("CARGO_PKG_VERSION"), " (+https://satchel.email)"))
            .timeout(Duration::from_secs(5))
            .build()?;
        Ok(self.http.get_or_init(|| c))
    }
}

#[async_trait]
impl PlaceSearch for Photon {
    fn name(&self) -> &'static str {
        "OpenStreetMap"
    }

    async fn suggest(&self, query: &str) -> Result<Vec<PlaceSuggestion>> {
        let r = self.http()?.get(&self.url).query(&[("q", query), ("limit", "6")]).send().await?;
        let status = r.status();
        if !status.is_success() {
            return Err(crate::net::http_error(status, &r.text().await.unwrap_or_default(), "place lookup"));
        }
        Ok(parse(&r.json().await?))
    }
}

/// Photon's GeoJSON features as suggestions, without repeats.
pub fn parse(v: &Value) -> Vec<PlaceSuggestion> {
    let mut out: Vec<PlaceSuggestion> = vec![];
    for f in v["features"].as_array().into_iter().flatten() {
        if let Some(s) = suggestion(&f["properties"])
            && !out.iter().any(|o| o.text == s.text)
        {
            out.push(s);
        }
    }
    out
}

fn suggestion(p: &Value) -> Option<PlaceSuggestion> {
    let get = |k: &str| p[k].as_str().map(str::trim).filter(|s| !s.is_empty());
    // House number first where that's the custom.
    let number_first = matches!(get("countrycode").map(str::to_ascii_uppercase).as_deref(), Some("US" | "CA" | "GB" | "AU" | "NZ" | "IE"));
    let street = match (get("street"), get("housenumber")) {
        (Some(s), Some(n)) if number_first => Some(format!("{n} {s}")),
        (Some(s), Some(n)) => Some(format!("{s} {n}")),
        (Some(s), None) => Some(s.to_string()),
        _ => None,
    };
    let label = get("name").map(String::from).or(street.clone())?;
    let area = get("city").or(get("locality")).or(get("district")).or(get("county"));
    let mut detail: Vec<String> = vec![];
    for part in [street.as_deref(), area, get("state"), get("country")].into_iter().flatten() {
        if part != label && !detail.iter().any(|d| d == part) {
            detail.push(part.to_string());
        }
    }
    // The field gets the name, street and town; the region only when there's no more.
    let mut text = vec![label.clone()];
    text.extend(detail.iter().take(if street.is_some() && street.as_deref() != Some(&label) { 2 } else { 1 }).cloned());
    Some(PlaceSuggestion {
        text: text.join(", "),
        label,
        detail: (!detail.is_empty()).then(|| detail.join(", ")),
        source: PlaceSource::OpenStreetMap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn photon_features_become_suggestions() {
        let v = json!({ "features": [
            { "properties": { "name": "Blue Bottle Coffee", "housenumber": "66", "street": "Mint Street", "city": "San Francisco", "state": "California", "country": "United States", "countrycode": "US" } },
            { "properties": { "name": "Berlin", "state": "Berlin", "country": "Germany", "countrycode": "DE" } },
            { "properties": { "housenumber": "12", "street": "Torstraße", "city": "Berlin", "country": "Germany", "countrycode": "DE" } },
            { "properties": { "name": "Berlin", "state": "Berlin", "country": "Germany", "countrycode": "DE" } },
            { "properties": { "country": "Nowhere" } },
        ] });
        let s = parse(&v);
        assert_eq!(s.len(), 3, "a repeat and a nameless place are dropped");
        assert_eq!(s[0].text, "Blue Bottle Coffee, 66 Mint Street, San Francisco");
        assert_eq!(s[0].detail.as_deref(), Some("66 Mint Street, San Francisco, California, United States"));
        assert_eq!(s[1].text, "Berlin, Germany");
        assert_eq!(s[2].label, "Torstraße 12");
        assert_eq!(s[2].text, "Torstraße 12, Berlin");
    }
}
