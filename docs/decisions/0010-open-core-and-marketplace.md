# 0010. Open core, paid services and the plugin marketplace

- **Status:** Accepted (2026-11-24, owner decision)
- **Affects:** D-80 (ADR 0009 plugins), ADR 0005, licensing, CONTRIBUTING
- **Detail:** service contract in [`docs/services/`](../services/README.md)

## Context

Ferro is public under MPL-2.0 and, so far, every commit is the owner's. The
owner asked whether to close part of the core for a competitive edge, and
decided: keep the core open, earn from services around it, and run a plugin
marketplace where plugin authors sell and Ferro takes a commission, paid
through VNPay (the owner already has a merchant account).

## Decision

1. **Open core.** Everything in this repository stays MPL-2.0: kernel seam,
   document model, file format, change format, app, CLI, SDK, MCP tools,
   plugin loader, and the *clients* of every service below. An open build
   works fully offline with no account.
2. **Closed services** (a separate private repository, never in this one):
   - account and identity;
   - license server (issues signed license tokens);
   - plugin marketplace backend (catalog, payments, payouts);
   - cloud sync and collaboration;
   - hosted AI agents;
   - heavy compute (render, energy, point cloud);
   - enterprise features (SSO, admin, audit).
   Payment-provider keys (VNPay `vnp_HashSecret`, terminal code) live only
   there.
3. **One service seam.** The app reaches services only through a configured
   base URL (`services.json` in the Ferro folder; default: the official
   endpoint; empty: offline). The protocol is open and documented in
   `docs/services/`, so anyone can run a compatible server, but the official
   one is ours.
4. **Entitlements by signed license tokens.**
   - **Issuing:** the license server issues compact tokens signed with
     Ed25519, listing the account, the entitlements (`plugin:<id>`,
     `cloud.sync`, `revit.link.pro`, …), expiry and device limit.
   - **Checking:** the app checks tokens offline with the public key built
     into the open code.
   - **Honest-user model:** like the SketchUp Extension Warehouse or Revit
     add-ins. An open client can be patched to skip checks; we accept that.
     The value is the service, updates, support and the marketplace's
     reach, not DRM.
5. **Marketplace.**
   - **Listing:** a plugin listing adds `publisher`, `price` (free /
     one-time / subscription, in VND), and an entitlement id to the plugin
     manifest.
   - **Buying:** the app shows Buy for a paid plugin it lacks a token for.
     Checkout happens on the web (VNPay); the token comes back by sign-in
     sync or an activation code.
   - **Payouts:** the platform collects the full amount and pays authors out
     on a schedule, minus the commission rate. The rate is set by the
     platform, not in this repo.
   - **Free plugins:** they stay free to install from anywhere (ADR 0009).
     The marketplace is one source among others.
6. **CLA.** From now on, outside contributions need a Contributor License
   Agreement (`docs/legal/CLA.md`). It keeps the right to relicense and to
   offer commercial licences. CI checks it on every pull request. The owner
   and maintainers listed in the CLA file are exempt.
7. **Trademark.** "Ferro" and its logo are the project's marks. Forks may use
   the code under MPL-2.0 but not the name (`docs/legal/TRADEMARK.md`).
   Registering the mark is a legal step outside this repository.

## Alternatives considered

- **Close part of the core.** It would break the community and agent model
  (agents read the code to build tools and send pull requests) and win
  little: big competitors have their own cores.
- **AGPL-3.0 plus a commercial licence.** It blocks closed hosted forks, but
  businesses avoid AGPL and plugin authors would worry. It stays possible
  later thanks to the CLA.

## Consequences

- New open modules: service configuration, account client, license-token
  verification and entitlements, marketplace catalog client and Buy flow,
  plugin manifest pricing fields (M6-20 in the task tree).
- Business code and secrets never enter this repository. A lint check
  refuses obvious secrets such as `vnp_HashSecret` values.
- Legal steps (CLA wording, trademark filing, payment terms, e-invoices and
  tax for sellers) need a lawyer and an accountant. The files here are
  working drafts.
