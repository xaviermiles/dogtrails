# Stravata Trails (Rust)

A small Rust + Axum web app that recommends dog-friendly running trails. It ships with sample data and a clean API surface so you can swap in a real provider later (DOC, AllTrails, etc.).

## Quick start

```powershell
cargo run
```

Open `http://127.0.0.1:3000`.

## API

- `GET /api/trails` — filters on distance, effort, length, dog access, and difficulty.
- `GET /api/providers` — shows provider availability notes.

Example:

`/api/trails?effort=steady&length=medium&dog=allowed_or_partial`

## Data

Sample trails live in `data/trails.json`. Replace or generate this file from an approved API/data source.

## Integrations (future)

Strava and Garmin require OAuth. Put credentials in `.env` based on `.env.example` and add the OAuth flow when ready.

## Notes on data sources

This project intentionally avoids scraping third-party sites without permission. Use official APIs or open data feeds.
