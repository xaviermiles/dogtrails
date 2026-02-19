# Stravata Trails (Rust)

A small Rust + Axum web app that recommends dog-friendly running trails. It pulls live data from the OpenStreetMap Overpass API (a public API) and renders the UI server-side (no JavaScript required).

## Quick start

```powershell
cargo run
```

Open `http://127.0.0.1:3000`.

If Overpass is busy, you can set multiple endpoints:

`OVERPASS_URL=https://overpass-api.de/api/interpreter,https://overpass.kumi.systems/api/interpreter,https://overpass.nchc.org.tw/api/interpreter`

## API

- `GET /api/trails` — filters on distance, effort, length, dog access, and difficulty.
- `GET /api/providers` — shows provider availability notes.

Example:

`/api/trails?effort=steady&length=medium&dog=allowed_or_partial`

## Data

Trails are fetched at runtime via Overpass using dog access tags. Use the region selector or provide a custom bounding box.

## Integrations (future)

Strava and Garmin require OAuth. Put credentials in `.env` based on `.env.example` and add the OAuth flow when ready.

## Notes on data sources

This project intentionally avoids scraping third-party sites without permission. Use official APIs or open data feeds.
