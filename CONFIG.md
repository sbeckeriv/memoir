# Configuration

memoir is configured via `~/.memoir/config.toml`. All fields are optional — any omitted value falls back to the default shown below. You can also point memoir at a different config directory with `--config-dir <path>` or by setting `$MEMOIR_CONFIG_DIR`.

## `[application]`

```toml
[application]
host = "127.0.0.1"
port = 3000
ui_poll_secs = 30
custom_css = ""
```

| Key | Default | Description |
|---|---|---|
| `host` | `127.0.0.1` | Address the HTTP server binds to |
| `port` | `3000` | Port the HTTP server listens on |
| `ui_poll_secs` | `30` | How often the UI refreshes stats and lists. Set to `0` to disable |
| `custom_css` | `""` | CSS injected into every page. See `HTML.md` for available classes |

## `[browser]`

```toml
[browser]
kind = "orion"
history_db_path = "~/Library/Application Support/Orion/Defaults/history"
```

| Key | Default | Description |
|---|---|---|
| `kind` | `orion` | Browser to read history from. One of: `orion`, `chrome`, `brave`, `arc`, `edge`, `chromium` |
| `history_db_path` | *(Orion default)* | Full path to the browser's SQLite history file |

## `[data]`

```toml
[data]
dir = "~/.memoir"
```

| Key | Default | Description |
|---|---|---|
| `dir` | `~/.memoir` | Directory where `index.db` and `config.toml` are stored |

## `[fetch]`

```toml
[fetch]
delay_ms = 200
timeout_secs = 15
user_agent = "memoir/0.1 (personal history indexer)"
max_retries = 3
ban = []
```

| Key | Default | Description |
|---|---|---|
| `delay_ms` | `200` | Milliseconds to wait between outbound requests |
| `timeout_secs` | `15` | Per-request timeout in seconds |
| `user_agent` | `memoir/0.1 …` | User-Agent header sent with fetch requests |
| `max_retries` | `3` | Times to retry a failed fetch before marking it as an error |
| `ban` | `[]` | List of hosts or host/path prefixes to skip (see below) |

### Ban patterns

Each entry in `ban` is matched against every URL memoir considers fetching:

- **Host-only** (no `/`) — matches the exact host and all subdomains.
  `"gmail.com"` blocks `gmail.com`, `mail.gmail.com`, etc.
- **Path-prefix** (contains `/`) — matches URLs whose path starts with the given prefix, but not sibling paths.
  `"github.com/myorg"` blocks `github.com/myorg/private` but not `github.com/otherorg`.

### Suggested ban list

Search engine result pages and login-walled services produce no useful content when fetched. Add any of these to your `ban` list as needed:

```toml
[fetch]
ban = [
  # Search engines — result pages are ephemeral and contain no useful content
  "google.com",
  "bing.com",
  "yahoo.com",
  "duckduckgo.com",
  "brave.com",
  "kagi.com",
  "yandex.com",
  "baidu.com",
  "ecosia.org",
  "startpage.com",

  # Email
  "mail.google.com",
  "outlook.live.com",
  "outlook.office.com",
  "mail.yahoo.com",
  "mail.proton.me",
  "fastmail.com",
  "hey.com",

  # Social / feeds — login-walled or ephemeral content
  "facebook.com",
  "instagram.com",
  "twitter.com",
  "x.com",
  "linkedin.com",
  "reddit.com",
  "tiktok.com",
  "snapchat.com",
  "threads.net",

  # Messaging
  "web.whatsapp.com",
  "messages.google.com",
  "telegram.org",
  "discord.com",
  "slack.com",

  # Cloud storage / docs (personal files, not public content)
  "drive.google.com",
  "docs.google.com",
  "sheets.google.com",
  "slides.google.com",
  "onedrive.live.com",
  "dropbox.com",
  "notion.so",

  # Banking / finance
  "chase.com",
  "bankofamerica.com",
  "wellsfargo.com",
  "citi.com",
  "capitalone.com",
  "schwab.com",
  "fidelity.com",
  "vanguard.com",
  "paypal.com",
  "venmo.com",

  # Shopping / account dashboards
  "amazon.com/gp",
  "amazon.com/your-account",
  "amazon.com/orders",

  # Internet Archive — large and rarely useful as personal memory
  "web.archive.org",
]
```

## `[llm]`

```toml
[llm]
provider = "lm_studio"
base_url = "http://localhost:1234"
model = "local-model"
max_context_chars = 8000
# api_key = ""
# system_prompt = ""
```

| Key | Default | Description |
|---|---|---|
| `provider` | `lm_studio` | One of: `lm_studio`, `openai`, `anthropic` |
| `base_url` | `http://localhost:1234` | API base URL. Ignored for `anthropic` |
| `model` | `local-model` | Model name passed to the API |
| `api_key` | *(unset)* | API key. Omit for local models |
| `max_context_chars` | `8000` | Total characters of page content sent to the LLM per query (~2k tokens) |
| `system_prompt` | *(built-in default)* | System prompt prepended to every `/ask` query. The current date is always prepended automatically. Omit to use the built-in prompt |

## `[sync]`

```toml
[sync]
interval_mins = 60
fetch_batch = 500
embed_batch = 200
```

| Key | Default | Description |
|---|---|---|
| `interval_mins` | `60` | How often the background sync runs, in minutes |
| `fetch_batch` | `500` | Max URLs fetched per sync cycle |
| `embed_batch` | `200` | Max pages embedded per sync cycle |
