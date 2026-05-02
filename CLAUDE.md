# memoir — Claude context

Personal browser history indexer. Rust/Axum backend, vanilla JS frontend, SQLite storage. Runs entirely on-device; no cloud, no telemetry.

## Commands

```sh
cargo build                  # build library + CLI
cargo build --release
cargo test                   # integration tests (spins up a real server)
cargo check                  # fast type-check, no binary
cargo run -- --no-sync       # start server without background sync
cargo run -- sync            # one-shot sync
```

Tests live in `tests/api/` and use a real `Application` instance — no mocks, no fakes. `tests/api/helpers.rs` contains the test harness.

## Project layout

```
src/
  lib.rs              — public re-exports
  main.rs             — CLI entry point, arg parsing, startup sync, periodic sync loop
  config.rs           — Settings structs + TOML load/save + ban pattern matching
  sync.rs             — sync loop: reads browser history, fetches pages, embeds
  session_log.rs      — in-memory log for the current server session
  browser/            — browser history readers (Orion, Chromium)
  embed/              — fastembed wrapper (BAAI/bge-small-en-v1.5, 384-dim)
  fetch/              — reqwest page fetcher + HTML text extractor (scraper)
  index/              — IndexStore: rusqlite wrapper, FTS5, vector BM25+cosine search
  rag/                — LlmClient: OpenAI-compat + Anthropic chat completions
  cluster/            — time-proximity session clustering
  mcp/                — MCP stdio server
  server/
    mod.rs            — AppState, Application, axum Router wiring
    handlers.rs       — all HTTP handlers

src/ui/               — embedded HTML pages (single-file, inline CSS + JS)
  index.html          — home: search, ask, recent, starred, clusters  (body.page-home)
  manage.html         — browse/star/delete/ban index entries            (body.page-manage)
  settings.html       — settings form                                   (body.page-settings)
  log.html            — activity log                                    (body.page-log)
  palette.html        — floating quick-search overlay                   (body.page-palette)
  setup.html          — first-run wizard                                (body.page-setup)

src-tauri/            — Tauri desktop app wrapper (macOS menu bar)
tests/api/            — integration tests against a live server
```

## Key types

- `Settings` / `FetchSettings` / `LlmSettings` — config structs, all `#[serde(default)]`
- `IndexStore` — wraps a rusqlite connection; methods for upsert, FTS5 search, vector search, starred, ban
- `AppState` — axum shared state: `browser`, `index`, `embedder`, `llm`, `config`, `sync_paused`, `palette_hide`, `log`
- `Application` — builds the router, binds the listener; `run_until_stopped` starts axum serve
- `SessionLog` — `Arc<SessionLog>` passed to sync and handlers; stores recent log entries in a `Mutex<VecDeque>`
- `AskBody` — POST body for `/api/ask`; `sources: Vec<AskSource>` lets the frontend pass already-visible results so the backend skips its own search

## Frontend conventions

- All UI is single-file HTML with inline `<style>` and `<script>` — no build step, no bundler
- `/api/custom-css` is loaded as the last stylesheet on every page so user CSS overrides built-ins
- Each `<body>` has a page-specific class (`page-home`, `page-manage`, etc.) for CSS scoping
- Search uses 300 ms debounce for live results; appending `?` to a query runs search then ask sequentially (ask receives the visible result list as `sources`, skipping a second server-side search)

## Conventions

- Axum extractors: `State(state): State<AppState>` always first, then `Query` or `Json`
- New routes go in `build_router` in `server/mod.rs` and the handler in `server/handlers.rs`
- Ban patterns: host-only (no `/`) matches host + all subdomains; with `/` it's a path-prefix match — see `matches_ban_pattern` in `config.rs`
- The embedding model is optional (`Option<Arc<dyn EmbedText>>`); all code paths handle `None`
- `sync_paused: Arc<AtomicBool>` — checked before each periodic sync cycle, not mid-sync
- Settings are re-read from disk each sync cycle so changes apply without restart

## Docs

- `CONFIG.md` — full config reference with suggested ban list
- `HTML.md` — CSS class reference for custom CSS
