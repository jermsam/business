# Workspace access implementation

Implemented 2026-10-08. This extends the existing Business server; it exposes only the fixed operations listed below, never arbitrary TypeQL.

## Database model

`migrations/002-workspace-access.tql` adds products, organization memberships, entitlements, teams, project ownership, grants and shared records. Required relation roles have explicit cardinality. Each team, project and shared record has exactly one ownership relation. New products/projects/records have stable keyed IDs, and grants/lifecycle relations have keyed IDs. Migration 004 backfills stable UUIDs for users/companies, canonical organization/person membership keys and stable private-record owner IDs. Authentication now issues stable UUID subjects; legacy email-subject tokens are rejected. Domain remains an optional request alias; clients should use the returned tenant UUID.

`workspace-functions.tql` implements membership, entitlement, direct/team grant evaluation and deny precedence. `migrations/003-account-lifecycle.tql` adds the public policy entry points:

- `biz_authorized`: account/organization/product lifecycle, selected-tenant membership, product entitlement and project action grant, with deny overriding allow.
- `biz_private_authorized`: active canonical membership plus account/organization lifecycle, preserving the private API's additional stable-creator-ID query filter.
- `biz_account_active`: used during authentication and token revalidation as well as data operations.

Shared membership, entitlement, team membership and grants require explicit active state and start time; end is optional. Start is inclusive; end is exclusive. Times are UTC represented as zone-free TypeDB datetime, supplied by the server. For legacy compatibility only, absent account/organization/product state is accepted; explicit suspension denies. Nested teams and resource inheritance are unsupported rather than inferred from the old IAM functions. The old generic IAM functions are not used by the new APIs.

Every shared query binds the selected company, and every team grant checks that team's owner against the same company. Cross-tenant grant rows cannot bridge that boundary. The policy is called inside each data operation's transaction. Schema constraints alone do not enforce all tenant equality rules; policy queries enforce those boundaries even when malformed grants were inserted by a privileged database client.

## Public API

All routes require a valid bearer token and `x-tenant-id` (prefer stable organization UUID; current domain aliases also resolve). Database credentials remain server-side.

| Route | Behavior |
|---|---|
| `GET /apps` | Distinct products with at least one project the caller can read. Entitlement alone is insufficient. |
| `GET /apps/{product-uuid}` | The caller's readable projects for that product. Returns 404 if none are visible. |
| `GET /workspace-records` | Readable records across authorized projects in the selected organization. |
| `POST /workspace-records` | Accepts `name` and `project_id`; requires project `create` permission. Server assigns record ID and creator. |
| `GET /workspace-records/{uuid}` | Requires project `read`. |
| `PUT` / `PATCH /workspace-records/{uuid}` | Accepts name only; requires project `update`. Project/creator cannot be reassigned. |
| `DELETE /workspace-records/{uuid}` | Requires project `delete`; removes the ownership relation and record together. |
| `GET /memberships` and `GET /memberships/{person-uuid}` | Organization owners can inspect canonical memberships. |
| `POST /memberships` | Owner-only creation with `person_id` and `role` (`member` or `owner`); canonical key prevents duplicate creation. |
| `PATCH /memberships/{person-uuid}` | Owner-only `role` and `state` replacement; cannot remove/demote/suspend the final eligible owner. |
| `PATCH /access-grants/{id}` | Owner-only grant state change, scoped to the organization. |
| `PATCH /entitlements/{id}` | Owner-only suspension. Activation and purchase verification require trusted provisioning; owner reactivation is rejected. |
| `PATCH /team-memberships/{id}` | Owner-only team membership state change; team must belong to the organization. |
| `/records` and `/records/{uuid}` | Existing private records remain creator-only; shared grants never expose them. |

Lists are bounded to 100, sorted by ID, and policy-filtered before limiting. There is no pagination yet. Names use the existing restricted ASCII name validator. Current query construction permits only validated identifiers/names and fixed server-generated values; typed `given` inputs are still a future improvement. No caller may supply a query, identity, tenant ownership, evaluation time or authentication override.

An inaccessible record and a nonexistent record both return 404. Invalid/revoked credentials or account/organization suspension return 401. Internal service calls verify the same bearer credentials. Product suspension, missing entitlement or denied project access cannot be bypassed through direct URLs.

## Explicit migration

Normal server startup never changes the database. Existing databases need all access schema/functions installed before running this revision, including for existing authentication/private-record operations.

```sh
BUSINESS_ENV_FILE="$HOME/.config/jitpomi/business/dev.env" \
  cargo run --locked --bin database -- migrate-access
```

The command permits `business_dev` only. It applies additive type definitions, defines/redefines functions and backfills identities/ownership in one schema transaction; failures close the transaction without committing. Re-running has been exercised on the live development database. Do not run two schema deployment commands concurrently. Fresh provisioning includes the same schema and function files. Production migration/deployment has not been performed; plan a protected export and restore check before enabling it there.

## Validation

Run the unit and live HTTP suites serially on the free development database:

```sh
BUSINESS_ENV_FILE="$HOME/.config/jitpomi/business/dev.env" \
  cargo test --locked -- --include-ignored --test-threads=1
```

`hosted_workspace_policy` creates unique synthetic fixtures, refuses other database names, and uses the actual Axum/DogRS router. It covers direct/team access, action boundaries, product entitlement, multiple organizations, malformed cross-tenant grants, lifecycle changes, deny precedence, protected lists, project discovery, authority-field injection, internal authentication forgery, authorized CRUD and logout. Existing auth and private-record suites remain required. Synthetic fixtures are retained for inspection, not mixed into production.

## Earlier recorded result — 2026-10-08

On the real free TypeDB Cloud `business_dev` database with Rust driver 3.13.6, the final serial test run passed **8 tests, 0 failures, 0 ignored**, including all three hosted HTTP suites, in 124.31 seconds. `cargo clippy --locked --all-targets -- -D warnings` and formatting checks passed. The access migration succeeded twice consecutively after its rerun handling was corrected. No production database or public deployment changed.

The source fingerprint file beside this document records the reviewed implementation. Test-source formatting and documentation were finalized after the run; no runtime behavior changed afterward. The hosted server version was not independently inspected in this turn.

## Identity, uniqueness and concurrency audit

See [identity and concurrency audit](identity-concurrency-audit.md) for the follow-up changes and validation. The earlier eight-test result above predates this audit.

Canonical memberships use `tenant-uuid:person-uuid` as their database key. Policies independently check that key against the linked participants, so a privileged client's malformed alternate-key row cannot become an effective membership. The migration refuses duplicate pairs instead of guessing which lifecycle state to preserve. It also rejects noncanonical identity UUIDs and dangling stable private-record owner references. It marks legacy memberships as migrated so a rerun does not recreate later-revoked memberships.

All protected record writes and supported administrative mutations change the selected tenant's `biz_revision` ownership in their transaction. This coordinates writes across backend instances through database conflicts. A stale overlapping write cannot commit after a coordinated grant revocation commits. Unrelated tenants use different revision ownerships; writes within one tenant can conflict even on different records. Protected writes use a shared executor that consumes and validates exactly one result before committing. Zero-result or ambiguous-result writes close without commit, including mutations that buffered changes before returning no rows. Database write transactions have a 30-second server deadline; transaction opening, result collection and commit acknowledgement each have a 30-second client bound. Cleanup has a two-second client bound. There is no automatic retry. An unconfirmed commit returns HTTP 503 with instructions to reconcile resource state before retrying; it does not claim the write was rolled back. These limits are write-specific, not a whole-request latency guarantee.

## Deliberate boundaries

The first organization owner is established through trusted provisioning; ordinary members cannot self-promote. Invitations, grant creation/expiry editing, grant audit history, SSO handoff and organization/account/product suspension APIs are not implemented. Schema migrations and privileged direct database edits must be coordinated separately; they do not automatically participate in the runtime revision protocol. Existing direct fixture edits are test setup, not recommended administration.

Reads may finish using an earlier snapshot. Logout rejects subsequent requests but does not cancel already-running work. The demonstrated stronger write coordination applies to the protected data and administrative write paths described above, not arbitrary database clients. This is not a general serializable database guarantee or a capacity certification.

User/company UUID attributes remain optional at schema level for additive migration compatibility; incomplete identities fail authentication. Provisioning must assign stable IDs before enabling a new identity. Existing private email/domain values are retained as historical metadata; permissions no longer depend on those values. The migration cannot reconstruct ownership history that had already been lost before the backfill.
