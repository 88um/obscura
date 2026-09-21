# Fork compatibility and upstream sync

This fork supports the Playwright integrations in reposter and HumorBank. The
2026-09-21 sync merges upstream `1a3169da276d7720732c7b20535474942917fb83`
into the integration line formerly at `d09f871` (`v0.4.0-reposter-identity.6`).
It retains shared history rather than rebasing published commits. The delta was
310 commits, including 172 non-merge commits, since the common base `d9ef40b`.

## Contracts retained

| Contract | Why the apps need it | Resolution |
| --- | --- | --- |
| `x-obscura-context` v1 | Reposter supplies stable account identity, browser profile and optional proxy/location | Retained; parsed after upstream Host/Origin/auth checks, with isolated connection contexts |
| `Network.getResponseBody` for fetch/XHR in stealth mode | Both apps parse Instagram JSON from Playwright response events | Retained shared body/event recorder for both transports |
| `Fetch.getResponseBody` and stable intercepted request IDs | Raw CDP interception must resolve the same request seen in `Fetch.requestPaused` | Retained; paused requests now target the enabling session on the owning page and carry exact POST bytes |
| GraphQL request headers, `postData`, base64 `postDataEntries` | Both apps replay cursor requests from captured templates | Retained; raw binary request bytes also survive CDP encoding |
| Events delivered to every page attachment | Playwright and raw CDP sessions can attach to the same page | Combined with upstream's all-live-page iteration |
| `data:` frame scripts | Frame bootstrap scripts must execute without an HTTP fetch | Moved into upstream's cancellation-safe frame work queue |
| Service-worker interface objects | Application bundles reference these interfaces during bootstrap | Retained stubs; this does not implement service-worker execution |
| `DOMStringMap` | Dataset brand checks must work | Replaced the duplicate fork implementation with upstream's constructor/prototype behavior |
| Stealth navigation callbacks | Response listeners must also see stealth navigation | Upstream now supplies this behavior; redundant fork code removed |
| Native ARM64/x86-64 Linux stealth archives and SHA256 files | Existing app Dockerfiles consume this artifact layout | Retained native release workflow; added identity/interception/frame gates |

The identity protocol still accepts only the supported `chrome_145`/Windows
profile. A supplied timezone must still match the process `TZ`; the upstream
merge does not add independently configurable per-account timezones. Proxy credentials remain excluded from diagnostic
messages. The new upstream V8 version does not mean the advertised identity
profile should silently change for existing accounts.

## Benefits available after the sync

- Concurrent pages keep independent live isolates, evaluation handles and
  execution contexts. This directly improves multiple-page Playwright sessions.
- Disconnected CDP clients release their contexts and memory. This matters for
  repeated ingestion jobs and account reconnects.
- Fetch/XHR support binary request bodies, binary XHR responses, `Response.body`
  and `bodyUsed`; interception preserves binary fulfill bodies and header
  overrides. The fork additionally preserves binary cached response bodies.
- Fetch follows final redirect identity and applies improved CORS/preflight and
  cross-origin credential stripping. Avoid relying on the former permissive
  cross-origin behavior in replay scripts.
- Stealth ES modules and screenshot resources use the page's configured
  transport, proxy and credentials. `OBSCURA_BLOCK_TRACKERS=0` can disable the
  stealth client's tracker filter when it blocks a required application request;
  leave it enabled unless that problem is demonstrated.
- `Input.insertText`, textarea selection, accessible names, visibility, labels
  and live form rendering are more compatible with browser automation.
- MCP exposes script network history and console messages; the live-view tool
  can follow agent targets. These are useful debugging tools, not required for
  the current ingestion paths. Screenshots, screencasts and PDF already existed
  at the old integration base; they are not new features introduced by this sync.
- Upstream upgrades `deno_core` from 0.350 to 0.412 and updates V8/rustls along
  with network, filesystem and resource-limit hardening. The new
  `release-dist` profile is available if smaller release artifacts are wanted;
  this fork keeps its previously tested `release` packaging profile.

## Deployment migration required

Do not replace the pinned binary in either app without updating CDP connection
configuration. Upstream now rejects browser Origin headers and unexpected Host
headers, optionally requires a bearer token, and refuses non-loopback serving
without a token of at least 32 bytes. This sync retains those protections.

The current sidecars forward public port 9222 through `socat` to loopback 9223.
A request with Host `obscura:9222` is no longer accepted by a default loopback
listener on 9223. The straightforward migration is to remove the extra forwarder
and run `obscura serve --host 0.0.0.0 --port 9222 --workers 1 --stealth` inside the
container with `OBSCURA_CDP_TOKEN` set from a secret. Keep the Docker service
internal, or publish only a host loopback port when local access is required.

Clients must send `Authorization: Bearer <token>` on BOTH `/json/version`
discovery and the WebSocket connection. Reposter's `x-obscura-context` header
can coexist with Authorization. HumorBank performs its own HTTP discovery before
`connectOverCDP`, so adding the header only to Playwright is insufficient.
Reposter's session-verification and content-mutation connections also need the
same auth configuration, not only its primary scraper connection.

`DOM.setFileInputFiles` now requires `--allow-file-access`; enable it only for a
sidecar that intentionally reads local upload files. It is separate from
`--allow-private-network`.

Inspected source pins:

- HumorBank v2: Dockerfile pins `v0.4.0-reposter-identity.6` and per-architecture
  archive checksums.
- Reposter local production-tester source: Compose defaults to
  `v0.4.0-reposter-identity.1`; Dockerfile fallback is `v0.3.0`. Runtime environment
  overrides may differ. These are source findings, not verified deployed state.

No application pins, secrets, containers or deployments are changed by this sync.
Create a new fork-specific release tag and replace the app pins/checksums only
alongside the auth migration. Do not reuse upstream `v0.2.0` through `v0.2.3`:
those names already refer to different commits in this fork.

## Postgen rendering assessment

Postgen currently launches Chromium or connects to a Playwright browser server.
That uses Playwright's native protocol, whereas Obscura exposes CDP. Changing a
binary path alone is insufficient: acquisition must use `connectOverCDP`, and
persistent-server lifecycle/health checks must understand the chosen backend.

Its real rendering pipeline also depends on `exposeBinding` for the Node-side
headline solver, embedded fonts and images, canvas text measurements, Range
geometry, multi-pass layout settlement, CSS filters, and element screenshots.
The existence of Obscura screenshots/PDF is not evidence that these produce the
same social-post artifacts. Keep Chromium until representative fixtures and then
the full Postgen gold matrix pass the existing thresholds. Do not regenerate gold
images or relax thresholds to declare a renderer migration successful.

The 2026-09-21 assessment ran four existing fixtures through Postgen's actual
pipeline with an injected CDP connection: `druk-bubble-v1/headline-only`,
`tweet-curve-v1/single`, `imsg-thread-light-v1/card`, and
`gradient-quote-v1/gradient-quote-sub`, all at 1080x1350 and device scale 1.
Chromium matched all four checked-in gold images exactly. Obscura timed out at
`page.setContent(html, { waitUntil: "load" })` on all four (15-second bounded
wait), before settlement or visual comparison could complete. This is a concrete
compatibility blocker, not a visual-parity verdict. Postgen should retain Chromium.
No gold images or Postgen source files were changed.

## Repeatable checks

Use release-mode nextest with at most two test processes. The release workflow
also gates the fork-specific identity, request encoding and frame behavior on
both native Linux architectures.

The bounded Playwright smoke script starts one browser process, uses two pages,
serves only local fixtures, and kills its owned process after 60 seconds:

```sh
# Supply an existing playwright-core installation; no app service is started.
NODE_PATH=/path/to/node_modules OBSCURA_BIN="$PWD/target/release/obscura" \
  node scripts/ci/fork-playwright-smoke.cjs
NODE_PATH=/path/to/node_modules OBSCURA_BIN="$PWD/target/release/obscura" \
  SMOKE_STEALTH=1 node scripts/ci/fork-playwright-smoke.cjs
```

Live account login, collection, publishing and archiving must be qualified
separately after migrating the sidecars. Offline compatibility tests do not
establish current Instagram behavior.

## Rendering and benchmark limits found during qualification

The original companion obstacle course reports 32/33 because its observer
fixture expects repeated notifications for a sentinel that remains intersecting.
Chromium also stops at ten cards on that fixture. Re-observing the sentinel after
each appended batch and scrolling it into view produces `io:50` in both engines;
this corrects the fixture without restoring the old always-visible observer shim.

The deterministic rendering run produced all screenshots and passed the Obscura
behavior assertions. Four Chromium assertions failed on this macOS host: one
Liberation Serif line-height check and three native input/textarea geometry
checks. Their expected dimensions are not portable across the host's fonts and
native controls. No expected dimensions or pixel thresholds were relaxed.

Fifteen representative sites were captured at both top and bottom against the
previous fork and Chromium. Ten top captures and eleven bottom captures met the
harness's comparison eligibility rules; the others were excluded for unstable
capture state. Visual inspection still shows typography, dropdown and layout
differences from Chromium, including MDN and Bootstrap. These captures are
qualification evidence, not a claim of Chromium rendering parity.

## v0.5.0 verification

- Full release-mode nextest: 1,796 passed, four skipped.
- Three render-resource regression tests: 20 consecutive iterations each passed.
  Fixture servers explicitly use blocking accepted sockets on macOS to prevent
  truncated font responses, and consume complete HTTP request headers.
- Both exact render and render+stealth release builds passed.
- Plain and stealth Playwright checks passed: cursor replay, response bodies,
  two live pages, flattened sessions, interception IDs, identity and bearer auth.
- Corrected companion obstacle course: 33/33. The original fixture limitation
  and Chromium comparison are described above.
