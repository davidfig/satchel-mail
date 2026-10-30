# 0050. Location suggestions: history first, place lookups opt-in

- **Status:** Accepted
- **Date:** 2026-10-30

## Context
An event's location was a plain text box. Most locations repeat (the
office, a regular café, a video link), and the rest are real places people
would rather pick than type. Looking a place up means sending what's typed
to someone else's server, which a local-first, private client shouldn't do
without asking. The research is in the vault note "Location Search for
Calendar Events".

## Decision
- `suggest_locations` (user only; not on the agent allow-list) returns
  places in this order:
  1. **History:** locations already used in events, most used first, then
     most recent. They're stored in a new `events.location` column (schema
     5, filled from existing events on upgrade). Always on, nothing leaves
     the device.
  2. **Place lookups:** only when the user turns on Privacy → Places
     (`look_up_places`, off by default), and only from 3 characters, after
     a 250 ms pause in typing. Answers are cached for 10 minutes; a slow or
     failed lookup just leaves the history suggestions.
- The host supplies the place service through `ProviderFactory::places()`,
  the same way as one-click unsubscribe, since the core has no HTTP client:
  - **OpenStreetMap, via Photon** (`photon.komoot.io`): built for
    search-as-you-type, no key, self-hostable. Requests identify Satchel
    in the User-Agent, and the list credits "© OpenStreetMap contributors".
    Nominatim was ruled out: its policy forbids autocomplete.
  - **Apple Maps in the Mac app**, through MapKit's
    `MKLocalSearchCompleter` (objc2-map-kit). No key, better places, and
    results near you. It runs on the main thread through Tauri's
    `run_on_main_thread`.
  - Google Places and Mapbox were ruled out: both need a paid key that an
    open-source client can't keep secret.
- The Privacy page names whichever service the host uses.

## Consequences
- Photon's public instance is best effort. The hosted version should run its
  own Photon and point `Photon::new` at it.
- Photon results aren't biased toward the user's area (no device location),
  so common names can match places far away. Apple Maps doesn't have this
  problem.
- Picking a place stores its text only, not coordinates, so there's no
  "Open in Maps" yet.
- `cargo run -p satchel-providers --example apple_places -- "query"` tries
  the MapKit code from a terminal.
