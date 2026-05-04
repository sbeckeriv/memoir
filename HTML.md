# Custom CSS reference

Custom CSS is applied to every page via the **Appearance → Custom CSS** field in Settings. It loads as the last stylesheet, so any rule you write overrides the built-in defaults.

## Page scopes

Each page sets a class on `<body>` so you can target a specific page:

| Class | Page |
|---|---|
| `body.page-home` | Search (home) |
| `body.page-manage` | Manage |
| `body.page-settings` | Settings |
| `body.page-log` | Activity log |
| `body.page-palette` | Quick-search palette (dark overlay) |

Example — increase font size only on the search page:
```css
body.page-home { font-size: 16px; }
```

---

## Shared elements

These classes appear on more than one page.

### `header`

Sticky top bar. Contains the app icon, navigation links, and (on some pages) an input.

```css
header { background: #1a1a2e; border-bottom-color: #333; }
```

### `.app-icon`

The memoir logo image in the header (`<img class="app-icon">`).

### `.header-nav`

The secondary navigation links in the header ("Manage · Settings · Activity"). Consistent across all pages.

### `.header-nav-current`

The non-link label for the current page within `.header-nav`.

### `.card`

White rounded panel. Used as the primary content container on the home, settings, and manage pages.

### `.card-title`

Small all-caps label at the top of a card.

### `.btn`

Base button style. Extended by modifier classes:

| Class | Use |
|---|---|
| `.btn-search` | Blue "Search" button (home page header) |
| `.btn-ask` | Purple "Ask" button (inside results card) |
| `.btn-primary` | Blue save/submit button (settings page) |
| `.btn-secondary` | Grey secondary button (settings page) |
| `.btn-prev` / `.btn-next` | Pagination buttons (manage page) |
| `.btn-pause` | Pause/resume sync button (manage page) |
| `.btn-sync` | "Sync Now" button (manage page) |

### `.icon-btn`

Small borderless icon/emoji button used for per-item actions.

| Modifier | State |
|---|---|
| `.icon-btn.starred` | Amber colour when the item is starred |
| `.icon-btn.del:hover` | Red hover state for delete buttons |
| `.icon-btn.ban-host:hover` | Yellow hover state for ban-host buttons (manage page only) |

### `.badge`

Inline pill label. On the manage page it is extended with status-specific colours:

| Class | Status |
|---|---|
| `.badge-fetched` | Green — page is indexed |
| `.badge-pending` | Indigo — queued to fetch |
| `.badge-auth_wall` | Amber — login wall detected |
| `.badge-error` | Red — fetch failed |
| `.badge-skip` | Grey — skipped |

### `.favicon`

16×16 site favicon `<img>`.

### `.empty`

Italic grey placeholder shown when a list has no items.

### `.spinner`

Animated circular loading indicator (hidden by default, shown via JS).

### `.toast`

Notification that briefly appears after an action.

- On the **settings page**: inline text beneath the save button. `.toast.error` changes the colour to red.
- On the **manage page**: fixed bottom-right overlay. `.toast.show` makes it visible.

---

## Home page (`body.page-home`)

### `.search-row`

Flex row containing a text input and a button. Used twice: once in the header for the main search, and once inside the results card for the ask input.

### `#results-section`

The results card (hidden until a search runs). Contains the ask row, ask output, and search results list.

### `.answer-text`

Purple-accented block that renders the LLM answer. Contains standard HTML elements (`p`, `ul`, `ol`, `h1`–`h3`, `code`, `pre`, `a`, `strong`) — you can restyle them all inside this selector.

```css
.answer-text { background: #1e1e2e; border-left-color: #a78bfa; color: #e2e8f0; }
.answer-text a { color: #a78bfa; }
```

### `.source-list`

List of source links rendered beneath the LLM answer.

### `.result-item`

One search result row. Contains:
- `.result-content` — title link + snippet + visit dates
- `.snippet` — excerpt with `<b>` highlights
- `.visit-dates` — "first visited / last visited" line
- `.result-actions` — star and delete `.icon-btn`s

### `.home-grid`

Two-column grid containing the Starred and This Week cards on the home page. Collapses to one column below 640 px.

### `.site-list` / `.site-item`

List and row used in the Starred card. Each `.site-item` contains a `.favicon` and a link.

### This Week card

| Class | Element |
|---|---|
| `.weekly-group` | One host group. Add `.collapsed` to collapse its pages |
| `.weekly-host` | Clickable host header row (favicon + hostname + badge + chevron) |
| `.weekly-toggle` | `▾` chevron that rotates when collapsed |
| `.weekly-pages` | List of page links beneath the host header |
| `.weekly-badge` | Pill label — `.weekly-badge.new` (green) or `.weekly-badge.active` (blue) |

### `.stats-bar`

Thin bar below the header showing index statistics (hidden until data loads).

### Cluster / Topics card

| Class | Element |
|---|---|
| `.cluster-list` | Container for all topic clusters |
| `.cluster-row` | One topic. Add `.open` to expand it |
| `.cluster-header` | Clickable header row |
| `.cluster-label` | Topic title text |
| `.cluster-score` | Coherence score pill — also has `.score-high`, `.score-mid`, or `.score-low` |
| `.cluster-meta` | Timestamp + duration text |
| `.cluster-toggle` | `▾` chevron icon |
| `.cluster-body` | Expanded content area (hidden unless `.cluster-row.open`) |
| `.cluster-domains` | Row of domain pills |
| `.domain-pill` | Individual domain tag |
| `.cluster-pages` | List of page links |
| `.cluster-page-link` | Individual page link |
| `.cluster-show-more` | "N more pages" expand button |
| `.cluster-actions` | Action buttons at the bottom of an expanded topic |
| `.cluster-action-btn` | Action button. `.cluster-action-btn.danger` turns red on hover |
| `.clusters-footer` | "Show N hidden topics" link beneath the cluster list |

---

## Manage page (`body.page-manage`)

### `.filter-row`

Flex row in the header containing the title/URL filter input.

### `.sync-card`

Horizontal status bar above the table showing sync state and controls.

### `.sync-status`

Left side of the sync card — dot + label + interval text.

### `.sync-dot`

8 px status dot. `.sync-dot.active` is green; `.sync-dot.paused` is amber.

### `.toolbar`

Row above the table containing status tabs, pagination, and export/import buttons.

### `.toolbar-info`

Right-aligned count text inside the toolbar ("Showing 1–50").

### `.status-tabs`

Container for the filter tab buttons.

### `.tab`

Status filter button ("All", "Indexed", etc.). `.tab.active` is the currently selected tab.

### Table columns

| Class | Column |
|---|---|
| `.col-fav` | Favicon column |
| `.col-title` | Title + URL column |
| `.col-status` | Status badge column |
| `.col-actions` | Action buttons column |

### `.title-cell`

Flex column inside the title cell containing:
- `.page-title` — primary title link
- `.page-url` — secondary URL link

---

## Settings page (`body.page-settings`)

### `.field`

Vertical label + input pair.

### `.field-pair`

Two `.field`s side by side in a two-column grid.

### `label .hint`

Light grey descriptive text beside a label.

### `.form-actions`

Row containing the Save button and the inline toast.

---

## Activity log (`body.page-log`)

### `.toolbar`

Filter bar at the top of the log.

### `.tab-btn`

Pill-shaped filter button. `.tab-btn.active` is the active filter.

### `.live-badge` / `.live-dot`

"Live" indicator in the toolbar. `.live-dot` pulses green.

### `.log-list`

Stacked list of log entries.

### `.log-entry`

One log row. Modifiers:
- `.log-entry.has-detail` — clickable, shows pointer cursor
- `.log-entry.expanded` — detail section is visible

### `.log-ts`

Relative timestamp on the left.

### `.log-kind`

Category pill. Extended by:

| Class | Category |
|---|---|
| `.log-kind-sync` | Blue — sync events |
| `.log-kind-llm` | Purple — LLM/ask events |
| `.log-kind-search` | Green — search events |
| `.log-kind-error` | Red — errors |

### `.log-message`

Main text of the log entry.

### `.log-detail`

Expanded detail block (hidden by default, shown when `.log-entry.expanded`).

### `.empty-state`

Centered placeholder when there are no log entries.

---

## Palette (`body.page-palette`)

The palette is a dark-themed overlay. Most elements are dark by default; override with care.

### `#palette`

The outer container with dark background, rounded corners, and shadow.

### `#search-row`

Input row at the top of the palette.

### `#query`

The search input. Text is white; placeholder is semi-transparent white.

### `.result-row`

One result in the list. `.result-row.is-selected` has a blue highlight.

### `.result-text`

Flex column containing:
- `.result-title` — page title (white)
- `.result-subtitle` — snippet or hostname (dim white)

### `#palette-footer`

Keyboard shortcut legend at the bottom.

### `.kbd`

Individual key hint badge inside the footer.
