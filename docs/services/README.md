# Ferro services: the open contract (ADR 0010)

The app talks to services only through this contract. The official server
is closed; the contract is open, so the client lives in this repository and
anyone can run a compatible server. All bodies are JSON (UTF-8). Times are
RFC 3339 UTC. Money is an integer number of đồng (VND, no decimals). Stable
names: once released, nothing here is renamed.

## 1. Configuration

`<ferro folder>/services.json` (the folder honours `FERRO_HOME`):

```json
{ "base_url": "https://services.ferro.example", "enabled": true }
```

- Missing file: the official endpoint, compiled in. `"enabled": false` or an
  empty `base_url`: offline, and every service call fails with
  `ferro.service.offline` without touching the network.
- `FERRO_SERVICES_URL` overrides `base_url` (tests, self-hosting). It never
  switches services back on when `"enabled": false`.
- A `services.json` that is not valid JSON, or a refused URL (below), gives
  offline mode too, with the reason shown in the account window: a broken
  configuration never sends anything anywhere.
- **TLS:** every URL the app calls or opens (`base_url`, `verification_uri`,
  `download_url`, `checkout_url`, screenshots) must be `https://`; plain
  `http://` is accepted only for `localhost`, `127.0.0.1` and `[::1]`. URLs
  with spaces, control characters or a `user@` part are refused. API calls
  do not follow redirects; downloads follow at most 5, never to `http://`.
  Local servers are reached without a proxy.
- Every request carries `User-Agent: Ferro/<version>` and, when signed in,
  `Authorization: Bearer <access token>`. The token is never sent to a
  `download_url`.
- **Status codes:** 2xx = success; 4xx (except 401) carry a JSON
  `{error, message?}` body (e.g. `authorization_pending`); 401 =
  `ferro.service.unauthorized` (the app refreshes once, then retries);
  3xx, a non-JSON body or a missing field = `ferro.service.protocol`; 5xx,
  connection, DNS and timeout failures = `ferro.service.unavailable`. Time
  limits: 20 s per call, 10 min per download; JSON answers up to 4 MB.
- Everything shown from the server is sanitised (control, bidirectional and
  zero-width characters dropped, length capped); ids used in URL paths must
  be `[A-Za-z0-9._-]{1,128}`.

The client is the `ferro-services` crate (blocking `ureq` + rustls, run on
worker threads by the app); it is not built for wasm.

## 2. Account (sign-in without a password field in the app)

The app never shows a password field. It uses the OAuth 2.0 device
authorization grant (RFC 8628):

1. `POST /v1/auth/device` → `{device_code, user_code, verification_uri, interval, expires_in}`.
2. The app shows `user_code` and opens `verification_uri` in the browser.
   The user signs in on the web.
3. The app polls `POST /v1/auth/token {device_code}` every `interval`
   seconds:
   - while pending: `{error: "authorization_pending"}`;
   - once signed in: `{access_token, refresh_token, expires_in, account: {id, name, email}}`.
   - `slow_down`: the app adds 5 s to the interval (RFC 8628 §3.5);
   - `access_denied` / `expired_token`, or `expires_in` passed: the sign-in
     ends (`ferro.service.protocol`), the user starts again.
   The app accepts `interval` from 1 to 60 s (default 5) and `expires_in`
   up to 3600 s; the device code is a credential and is never shown.
4. Refresh with `POST /v1/auth/refresh {refresh_token}` →
   `{access_token, refresh_token?, expires_in, account?}` (missing fields
   keep their old values); any non-2xx answer means "sign in again". The
   app refreshes when the access token has less than 60 s left, and once
   after a 401. Sign out with `POST /v1/auth/revoke {refresh_token}`
   (Bearer); the app forgets the tokens whatever the answer.

Tokens are stored in `<ferro folder>/account.json` with 0600 permissions on
unix (written through a temporary file; on Windows the user-profile ACL
applies). The OS keychain was not used for now: the `keyring` crate needs
D-Bus headers on Linux CI and could not be verified on the macOS and
Windows runners; it can replace the file later behind the same functions.
Tokens are never logged and never sent to MCP clients or plugins: the
client keeps them in a type that cannot be printed, and the app never reads
their text. A plugin still runs with the user's rights (ADR 0009), so it
could read `account.json` from disk like any program the user runs; the
honest-user model of ADR 0010 accepts this.

## 3. License tokens and entitlements

A license token is `base64url(payload) "." base64url(signature)`
(base64url without padding; padding is accepted):
- the signature is Ed25519 over the payload bytes, i.e. the decoded JSON
  bytes, not their base64url text (checked with `verify_strict`);
- the payload is JSON:

```json
{
  "v": 1,
  "sub": "<account id>",
  "iat": "2026-11-24T06:00:00Z",
  "exp": "2027-11-24T06:00:00Z",
  "ent": ["plugin:acme.stairs", "cloud.sync", "revit.link.pro"],
  "dev": 3,
  "kid": "2026-1"
}
```

- `kid` selects the public key. Public keys are built into the app (a list,
  so keys can rotate); the private key lives only on the license server.
  - **Where the real key goes:** `PRODUCTION_KEYS` in
    `crates/ferro-services/src/license.rs`, as `("<kid>", [32 bytes])`. It
    ships empty: the owner adds the licence server's *public* key there.
  - The only other entry is the clearly marked **DEVELOPMENT** key, `kid`
    `dev`: its public half comes from `FERRO_DEV_LICENSE_KEY` (base64url,
    printed by the mock server) and it is trusted **only while the services
    URL is on this machine**. No private key is in the repository; the mock
    server and the tests create theirs at run time.
- The app checks tokens offline:
  - signature, `exp`, and `v` must be valid; a token that is valid but
    expired keeps working for a 14-day grace period, with a warning (up to
    `exp` + 14 days inclusive, then `ferro.license.expired`);
  - entitlements of all valid tokens add up; ids outside `[a-z0-9.:_-]`
    are ignored;
  - `dev` (device limit) is enforced by the server only; `iat` is not
    compared with the clock.
- Endpoints:
  - `GET /v1/licenses` (signed in) → `{tokens: [..]}`: refreshed at start
    and after a purchase. The fetched tokens replace the cached tokens of
    the same account (`sub`), so a revoked token disappears; tokens of
    other accounts or of activation codes stay;
  - `POST /v1/licenses/activate {code}` → `{tokens}`: an activation code
    bought on the web, usable without staying signed in (sent with the
    Bearer token when signed in). An unknown code is a 4xx with a
    `message` the app shows.
- Cache: `<ferro folder>/licenses.json` `{"tokens": [..]}` (signed, so plain
  storage), checked again at every start, so the app works offline.
- Entitlement ids:
  - `plugin:<plugin id>`: a paid plugin;
  - `cloud.sync`, `cloud.storage`, `agents.hosted`, `render.cloud`,
    `energy.cloud`, `pointcloud.cloud`, `revit.link.pro`, `enterprise`.
  New ids are added here first.

## 4. Marketplace

- `GET /v1/market/plugins?query=&page=` → `{plugins: [listing], next}`.
- `GET /v1/market/plugins/{id}` → listing. A listing is:
  - `id`, `name`, `publisher {id, name, verified}`, `summary`;
  - `version`, `price` (see below), `entitlement`;
  - `download_url`, `sha256`, `size`, `screenshots`, `rating`, `installs`.
- `price`:
  - `{"kind": "free"}`;
  - `{"kind": "one_time", "amount": 490000}`;
  - `{"kind": "subscription", "amount": 99000, "period": "month"}`.
- **Install:**
  1. download `download_url`;
  2. check `sha256`;
  3. install like any plugin zip (ADR 0009 install screen).
  A digest mismatch deletes the download (`ferro.market.checksum`).
  Paid plugins also need their entitlement. Without it the plugin is
  installed but its commands show **Buy** ("(Buy)" after the label); running
  one fails with `ferro.license.missing` and opens its listing in the
  app's Marketplace window. A listing without `entitlement` and a price
  above 0 means `plugin:<id>`. `next` is the `page` value of the next page
  (text or number), or null. `rating` is 0 to 5.
- **Buy:** `POST /v1/market/checkout {plugin_id}` (signed in) →
  `{checkout_url, order_id}`.
  - The app opens `checkout_url` in the browser; the payment happens there.
  - The app polls `GET /v1/market/orders/{order_id}` → `{status: pending | paid | failed | cancelled}`
    every 3 s for at most 30 minutes (the user can stop waiting).
  - When `paid`, it refreshes licenses.
- **Sell:** authors publish from the web (publisher account, payout details,
  tax info). The open repo only defines the manifest fields a listing reads
  (ADR 0009 manifest + `publisher`, `price`, `entitlement`).

## 5. Payment flow on the server (VNPay), for the closed implementation

This section describes what the closed server must do. The app never sees
any of it.

- Checkout creates an order (amount, plugin, buyer). It builds a VNPay
  payment URL:
  - a unique `vnp_TxnRef` per attempt;
  - the amount in VNPay's format;
  - the return URL;
  - the request signed with the merchant's secret.
  Follow the official VNPay API documentation for the exact parameters and
  hash; check them against the sandbox before going live.
- **IPN handler (the only place that marks an order paid):**
  - verify the signature;
  - check that the order exists, that the amount matches the stored amount
    and that the order is not already settled;
  - be idempotent, because IPNs can repeat;
  - answer with VNPay's expected response codes;
  - on success, issue the license token and record the sale.
- **Return URL:** shows the user the result only and never fulfils, because
  the user can skip it.
- **Ledger:** every sale stores gross, commission (platform rate at sale
  time), net to the publisher, VNPay reference and status. Payouts run on a
  schedule from the ledger. Refunds reverse the entries and revoke the token.
- Reconcile with VNPay reports daily. Keep logs for audit. Secrets live in
  the server's secret store, never in code or in this repository.

## 6. Other paid services (clients come later, same seam)

| Service | Entitlement | Contract (to be specified before building) |
|---|---|---|
| Cloud sync / collaboration | `cloud.sync` | change format (M5-26) over `/v1/projects/{id}/changes` |
| Hosted agents | `agents.hosted` | MCP over HTTPS at `/v1/agents/mcp` |
| Render / energy / point cloud | `render.cloud`, `energy.cloud`, `pointcloud.cloud` | job API `/v1/jobs` (submit, status, result) |
| Revit Link Pro | `revit.link.pro` | the add-in checks the same license tokens |
| Enterprise | `enterprise` | SSO (OIDC), admin, audit |

## 7. Failure codes (client side)

| Code | Meaning |
|---|---|
| `ferro.service.offline` | Services are switched off (`services.json`); no network was touched |
| `ferro.service.unavailable` | The server did not answer (connection, DNS, timeout; this includes "no network") or answered 5xx |
| `ferro.service.unauthorized` | Not signed in, or the token expired and could not be refreshed |
| `ferro.service.protocol` | The answer did not follow this contract, a URL was refused (TLS rule), or the sign-in was refused / expired |
| `ferro.license.invalid` | A license token failed verification |
| `ferro.license.expired` | A token is past its grace period |
| `ferro.license.missing` | The entitlement for this plugin or feature is not owned |
| `ferro.market.checksum` | A downloaded plugin does not match its `sha256` |

## 8. Mock server (open, tests and local development)

`ferro_services::mock::MockServer` implements this contract in memory on
`127.0.0.1` only, with fake payments:

- device flow that approves itself after N polls (optionally one
  `slow_down` first); verification page `/device`;
- licences signed with a key created at start (the DEVELOPMENT key);
  activation code `FERRO-TEST-CODE` gives `cloud.sync`;
- a catalog with `mock-free` (free) and `mock-paid` (490,000 đồng,
  `plugin:mock-paid`), zips built in memory;
- checkout whose order becomes `paid` when its fake payment page
  `/pay/<order_id>` is opened; orders.

Run it and point the app at it:

```text
cargo run -p ferro-services --bin ferro-services-mock -- --port 8787
FERRO_SERVICES_URL=http://127.0.0.1:8787 FERRO_DEV_LICENSE_KEY=<printed key> ferro
```

The end-to-end tests (`crates/ferro-services/tests/e2e.rs`) run the whole
path against it: sign in, browse, install the free plugin, buy the paid
one, refresh licences, run the paid plugin; offline mode makes no request;
tampered and expired tokens; a wrong `sha256`; the TLS rule.
