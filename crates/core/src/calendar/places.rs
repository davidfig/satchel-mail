//! Place suggestions for an event's location (0050): places already used in
//! events, then, if the user turned it on, the host's place service.

use std::time::{Duration, Instant};

use anyhow::Result;
use satchel_api::*;
use tracing::warn;

use crate::engine::Engine;

/// Online lookups start at this many characters.
const MIN_LOOKUP: usize = 3;
/// Most suggestions from history, and in all.
const HISTORY: usize = 5;
const TOTAL: usize = 10;
/// How long a lookup's answer is reused, and how many are kept.
const CACHE_FOR: Duration = Duration::from_secs(600);
const CACHE_SIZE: usize = 200;
/// A slow place service doesn't hold up the history suggestions for long.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);

impl Engine {
    pub(crate) async fn suggest_locations(&self, query: &str) -> Result<Vec<PlaceSuggestion>> {
        let query = query.trim();
        let mut out: Vec<PlaceSuggestion> = self
            .store
            .location_history(query, HISTORY)?
            .into_iter()
            .map(|l| PlaceSuggestion { label: l.clone(), text: l, detail: None, source: PlaceSource::History })
            .collect();
        if self.settings().look_up_places && query.chars().count() >= MIN_LOOKUP {
            for p in self.look_up_places(query).await {
                if out.len() >= TOTAL {
                    break;
                }
                if !out.iter().any(|o| o.text.eq_ignore_ascii_case(&p.text)) {
                    out.push(p);
                }
            }
        }
        Ok(out)
    }

    /// The host's place service, cached; empty when it fails (history still shows).
    async fn look_up_places(&self, query: &str) -> Vec<PlaceSuggestion> {
        let Some(service) = self.factory.places() else { return vec![] };
        let key = query.to_lowercase();
        if let Some((at, hit)) = self.place_cache.lock().unwrap().get(&key)
            && at.elapsed() < CACHE_FOR
        {
            return hit.clone();
        }
        let found = match tokio::time::timeout(LOOKUP_TIMEOUT, service.suggest(query)).await {
            Ok(Ok(found)) => found,
            Ok(Err(e)) => {
                warn!("{} place lookup failed: {e:#}", service.name());
                return vec![];
            }
            Err(_) => {
                warn!("{} place lookup timed out", service.name());
                return vec![];
            }
        };
        let mut cache = self.place_cache.lock().unwrap();
        if cache.len() >= CACHE_SIZE {
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
            if cache.len() >= CACHE_SIZE {
                cache.clear();
            }
        }
        cache.insert(key, (Instant::now(), found.clone()));
        found
    }
}
