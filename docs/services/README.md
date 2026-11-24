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
- `FERRO_SERVICES_URL` overrides `base_url` (tests, self-hosting).
- Every request carries `User-Agent: Ferro/<version>` and, when signed in,
  `Authorization: Bearer <access token>`.

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
4. Refresh with `POST /v1/auth/refresh {refresh_token}`. Sign out with
   `POST /v1/auth/revoke`.

Tokens are stored in the OS keychain where available, else in
`<ferro folder>/account.json` with 0600 permissions. They are never logged
and never sent to MCP clients or plugins.

## 3. License tokens and entitlements

A license token is `base64url(payload) "." base64url(signature)`:
- the signature is Ed25519 over the payload bytes;
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
- The app checks tokens offline:
  - signature, `exp`, and `v` must be valid; a token that is valid but
    expired keeps working for a 14-day grace period, with a warning;
  - entitlements of all valid tokens add up.
- Endpoints:
  - `GET /v1/licenses` (signed in) → `{tokens: [..]}`: refreshed at start
    and after a purchase;
  - `POST /v1/licenses/activate {code}` → `{tokens}`: an activation code
    bought on the web, usable without staying signed in.
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
  Paid plugins also need their entitlement. Without it the plugin is
  installed but its commands show **Buy**.
- **Buy:** `POST /v1/market/checkout {plugin_id}` (signed in) →
  `{checkout_url, order_id}`.
  - The app opens `checkout_url` in the browser; the payment happens there.
  - The app polls `GET /v1/market/orders/{order_id}` → `{status: pending | paid | failed | cancelled}`.
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
| `ferro.service.offline` | Services are switched off or no network |
| `ferro.service.unavailable` | The server did not answer or answered 5xx |
| `ferro.service.unauthorized` | Not signed in, or the token expired and could not be refreshed |
| `ferro.service.protocol` | The answer did not follow this contract |
| `ferro.license.invalid` | A license token failed verification |
| `ferro.license.expired` | A token is past its grace period |
| `ferro.license.missing` | The entitlement for this plugin or feature is not owned |
| `ferro.market.checksum` | A downloaded plugin does not match its `sha256` |
