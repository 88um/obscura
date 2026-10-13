`--storage-dir` persists cookies and `localStorage` so they survive across runs.

## CLI

```bash
obscura fetch https://example.com --storage-dir ./obscura-data
obscura fetch https://example.com --storage-dir ./obscura-data
```

The second invocation starts with the cookies and `localStorage` left by the first.

## Server

```bash
obscura serve --storage-dir ./obscura-data
```

All CDP sessions read and write to the same directory. Run separate `obscura serve` processes with different `--storage-dir` paths for isolated profiles.

For MCP, put the profile flag before the subcommand:

```bash
obscura --storage-dir ./obscura-data mcp
```

The MCP server also accepts `OBSCURA_STORAGE_DIR` and flushes the profile on exit.

## Layout

Inside `./obscura-data`:

- `cookies.json`: cookie jar in a stable format with `same_site`, `expires`, `http_only`, `secure`.
- `localStorage/<origin>-<digest>.json`: one file per origin, holding
  `{"origin": "...", "entries": [["key", "value"], ...]}`. The digest is an FNV-1a
  hash of the origin, avoiding collisions caused by filename sanitization.
  This implementation retains entries in insertion order.

Inspect one with `jq`:

```bash
jq '.entries' ./obscura-data/localStorage/*.json
```

The format is stable. Inspect with `jq`:

```bash
jq '.[] | select(.domain == "example.com")' ./obscura-data/cookies.json
```

## When state is written

- After every navigation completes (CDP `Page.navigate`, MCP `browser_navigate`),
  and only when something actually changed.
- On clean process exit (Ctrl-C, SIGTERM).
- Manually via CDP `Network.setCookie` and `Network.deleteCookies`.

`sessionStorage` is deliberately not written to disk. It belongs to one tab and
is dropped when the tab goes away, the same as a real browser, but it does
survive navigation and CDP target switching within the process
([#678](https://github.com/h4ckf0r0day/obscura/issues/678)).

Web Storage is limited to 5 MiB per origin, 32 MiB per backing store and 256
origins. Failed profile writes and removals stay pending for a later flush.
The budgets count UTF-8 key/value bytes, excluding container overhead. Strings
containing unpaired UTF-16 units
or starting with NUL use a NUL-prefixed JSON string escape internally, including
in profile files; the JavaScript API decodes them losslessly. Normal strings are
stored unchanged. Encoded strings consume more of the byte budget.
Shared contexts serialize flushes with storage mutations. Use separate profile
directories for independent browser processes; this is not a cross-process
database. Opaque-origin documents cannot access either store.

Automatic cross-document `storage` events are not implemented by this change.

## Login once, scrape many

```bash
obscura serve --storage-dir ./session-1
```

Drive a login flow once via Puppeteer or Playwright. Stop the server. Subsequent runs against the same `--storage-dir` start logged in.

## Multiple identities

```bash
obscura serve --port 9222 --storage-dir ./identity-a
obscura serve --port 9223 --storage-dir ./identity-b
```

## Clear state

```bash
rm -rf ./obscura-data
```
