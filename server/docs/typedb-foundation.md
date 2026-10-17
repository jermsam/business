# TypeDB foundation for JITPOMI Business

Research date: 2026-10-08. This is a design reference, not a production certification or an implemented migration.

## Implementation checkpoint

The [workspace access implementation](workspace-access.md) now documents the additive schema, TypeQL policy functions, protected app/project discovery and shared-record API. The subsequent [identity and concurrency audit](identity-concurrency-audit.md) adds stable identity backfill, owner membership administration and tenant revision coordination for supported writes. Invitations, SSO, complete provisioning and capacity validation remain outside that tested scope. The design below remains the broader target.

## Follow-up research

The current [Business implementation contract](/Users/samsonssali/WebstormProjects/jitpomi.com/docs/typedb/business-implementation-contract.md) records the focused review, tenant policy contract, concurrency candidates and acceptance scenarios. The earlier coverage figures below are a historical snapshot; the website handbook maintains the expanded reading ledger. That research-only follow-up did not rerun tests; the later implementation audit linked above records the live validation.

## Scope and evidence

Read the TypeQL 3 summary, the eleven numbered chapters (including setup and epilogue) of RBAC in Business, and selected modeling, transaction, driver and operations references. [Reading coverage](typedb-reading-coverage.md) records 29 fully reviewed documentation pages, three partial reviews and the remainder of the 278-page documentation index. The pricing page was also reviewed. The entire website, blog archive, research papers and videos have NOT been reviewed. Local source snapshots are kept outside this application in the Codex workspace’s `typedb-study` directory.

The application currently pins `typedb-driver = 3.13.6` and published DogRS 0.3.0. Current online 3.x documentation is a moving reference, not proof that every described feature works on our deployed server. The cloud cluster’s version/configuration was not re-inspected during this research turn. No database, credentials, billing or runtime code changed.

## Agreed product direction

JITPOMI is the umbrella for shared identity, organizations, invitations, access, billing and navigation. Each established product retains its own website and workflows. A person enters through JITPOMI, chooses an organization, sees their available apps, then enters a product with that organization context. Direct entry through a product should resolve the same access decisions. StoryGallery is experimental, not an established product to build the umbrella around.

Keep useful free Cloudflare services while this foundation matures. TypeDB should own the agreed relationship data; it need not store binary files or replace every delivery, networking or storage service.

## Model relationships, not a growing set of flags

TypeDB has entities, first-class relations and attributes. Relations can connect multiple participants, own values and participate in other relations. These capabilities fit memberships, grants and their audit history. [Data model](https://typedb.com/docs/core-concepts/typeql/entities-relations-attributes/)

The schema-modeling guide recommends modeling independent concepts as entities, connections as relations, and values as attributes. It also explains that attributes are shared by type/value and relation roles need explicit cardinalities when participants are mandatory. [Modeling guide](https://typedb.com/docs/guides/schema-modeling/)

Proposed JITPOMI model — not yet implemented:

| Concept | Intended representation |
|---|---|
| Person/account | Stable opaque ID; email is a verified, changeable login/contact value |
| Organization | Stable opaque ID; domain and display name are changeable attributes |
| Product | Catalog entity, with stable key and lifecycle state |
| Membership | Person–organization relation, status and validity interval |
| Product entitlement | Organization–product relation with state, validity and subscription/provisioning provenance |
| Team | Organization-owned entity with membership relations |
| Project | Organization-owned entity, associated with the relevant product |
| Access grant | Organization, subject, target, grantor and role/action scope, with validity and provenance |
| Access review | Relation to the grant being reviewed, reviewer and outcome |

A person can be a JITPOMI employee and a collaborator elsewhere simultaneously. Model these contextual roles as relations, not mutually exclusive person subtypes. Product names and tenant names normally belong in data rather than new schema types for each customer or app. [Composition](https://typedb.com/docs/academy/9-modeling-schemas/9.5-composition-over-inheritance/), [schema versus data](https://typedb.com/docs/learn-typedb/rbac-in-business/03-populating-amasoft/)

Use explicit required participants and stable keys. A name marked `@key` is not automatically unique only within a tenant. Audit existing globally keyed group/role names before extending them to tenant-owned roles. Unique membership/grant identifiers must be designed deliberately; cardinality on each relation’s participants does not alone prevent duplicate relation instances. [Constraints](https://typedb.com/docs/core-concepts/typeql/constraining-data/)

## Authorization contract

Proposed evaluation inputs: verified principal ID, selected organization ID, product ID, optional resource/project ID, requested action and server-controlled evaluation time.

An allow decision should require an active account, active membership in that selected organization, an active organization entitlement, and an applicable individual/team/project permission. The target resource and every grant/team traversal must remain inside the same organization. Missing facts or policy errors deny access. Establish explicit precedence for deny/allow rules before exposing configurable policies.

The app launcher and each product endpoint should use the same policy definition. Hiding a tile is not enforcement. Sharing any organization somewhere in the graph is insufficient: the selected organization must bind the entire access path. Employment by JITPOMI is not a blanket grant over client companies.

TypeQL functions can encapsulate and compose read policies, including recursive queries. They do not automatically intercept every query or enforce access for a database credential. Business must invoke the policy in protected operations. [Functions](https://typedb.com/docs/core-concepts/typeql/functions/), [policies as functions](https://typedb.com/docs/learn-typedb/rbac-in-business/07-policies-as-functions/)

Store why a grant exists and who approved it; preserve time-bounded grant history and reviews rather than relying only on the current function definition. Define retention separately, rather than treating tutorial language about permanent history as our retention policy. [Accountable access](https://typedb.com/docs/learn-typedb/rbac-in-business/08-accountable-access/)

JITPOMI single sign-on across separate websites is additional HTTP/session work. TypeDB is the relationship store, not a browser SSO protocol. Product handoff needs verified product identity, safe redirects, session/audience rules and tenant revalidation; do not pass bearer tokens in URLs or infer access from an email domain.

## Database privileges are not tenant permissions

Current TypeDB documentation describes administrator and standard accounts; standard accounts can read/write all server databases. A separate non-admin runtime user therefore does not create per-tenant or per-database isolation. Keep database credentials server-side. Application identities and permissions are a different layer from TypeDB server users. [Users](https://typedb.com/docs/core-concepts/typedb/users/)

Existing Business dev/prod database separation is logical separation on one cluster. The earlier live record-isolation test proves the application paths it exercised, not resistance to a compromised database credential.

## Transactions, concurrency and provisioning

TypeDB documents snapshot isolation, not serializable transactions. Reads see a transaction snapshot. Writes can fail at commit. Schema transactions exclude concurrent writes and can combine schema/data changes atomically; they belong in explicit migrations. [Server transactions](https://typedb.com/docs/core-concepts/typedb/transactions/)

For Business, authorize and mutate within one short write transaction. That avoids an independent pre-check followed by a later mutation, but does not imply immediate cancellation of already-running operations when a grant is revoked. We must define revocation semantics and test concurrent privilege changes. Cross-record invariants such as last-owner protection or separation of duties need dedicated concurrency tests and an explicit coordination design; snapshot reads alone are not proof of enforcement.

`put` matches the whole supplied pattern; it is not a partial-field merge. Its reference explicitly warns about duplicate inserts from multiple inputs or concurrent transactions unless suitable constraints prevent them. Use stable idempotency keys, deduplicated inputs and bounded conflict handling for onboarding, invitations, entitlements and payment events. [Put reference](https://typedb.com/docs/typeql-reference/pipelines/put/)

Distinguish a known rolled-back conflict from an uncertain commit outcome. Do not blindly retry business effects after a timeout. External payment/email effects are not made atomic by a TypeDB transaction; use persisted event/receipt state and an idempotent delivery design.

## Safer queries and driver integration

Reuse the driver, close transactions promptly, resolve query failures and explicitly commit writes. Set request/transaction limits based on measured workloads. Documentation gives differing batch-size suggestions, so benchmark our actual operations instead of copying a universal number. [Driver best practices](https://typedb.com/docs/core-concepts/drivers/best-practices/), [driver transactions](https://typedb.com/docs/core-concepts/drivers/transactions/)

`given` declares typed input rows separate from query text. The installed 3.13.6 Rust source includes `GivenRows`, `Transaction::query_with_rows` and `query_with_options_and_rows`. This is a promising next improvement over our restricted string interpolation, but server execution and adapter support still need tests. Current `dog-typedb::TypeDBAdapter` read/write methods take a query string; do not claim typed rows are already wired through them. [Given rows](https://typedb.com/docs/typeql-reference/pipelines/given/)

Keep schema identifiers and query shape under application control. Test quotes, Unicode, backslashes and malformed IDs when introducing typed rows. Preserve validation of business meaning even when values are safely bound. Return explicit fields rather than fetching every attribute from an identity, which could include credentials.

TypeQL 3 differs substantially from TypeQL 2. Use the current reference for `links`, `select`, `fetch`, functions and schema changes; do not paste old rule syntax. Stream/scalar function calls and branch-local variables need careful checking. [TypeQL summary](https://typedb.com/docs/llms-full.txt), [invalid patterns](https://typedb.com/docs/core-concepts/typeql/invalid-patterns/)

## Operations and the free constraint

The current pricing page advertises a free Explore offering with 10 GB storage. That is not evidence that every Cloud feature or capacity is enabled on our existing cluster. The page currently lists 8 GB RAM whereas the earlier inspected cluster had 4 GB; re-check the account before making capacity claims. No plan changes are authorized by this study. [Pricing](https://typedb.com/pricing)

The architecture reference describes replicated read scaling and a per-database leader for writes. Do not assume adding replicas gives proportional write throughput, or that our free single-node deployment has multi-node recovery guarantees. [Horizontal scaling](https://typedb.com/docs/core-concepts/typedb/horizontal-scaling/)

TypeDB supports schema-plus-data export/import, with import to a new database. A completed export needs sufficient client disk space; an interrupted export can be unusable. Plan a protected export and actual restore drill before migrations of valuable data. Generic Cloud backup documentation is not proof that managed backups are included on our free plan. [Export/import](https://typedb.com/docs/maintenance-operation/database-export-import/), [backups](https://typedb.com/docs/maintenance-operation/typedb-backups/), [upgrades](https://typedb.com/docs/maintenance-operation/typedb-upgrades/)

## Documentation discrepancies and example limits

Record these instead of silently treating every page as exact:

- RBAC chapter one describes unconstrained counts loosely; the schema-modeling reference specifies defaults of 0..1 for owns/relates and 0.. for plays. Set explicit critical constraints and validate on our version.
- Server transaction docs state a 10-second schema-lock default; driver transaction prose states 30 seconds. Configure needed limits explicitly rather than relying on either prose default.
- The Cloud install page says replication is coming soon, while the horizontal-scaling page describes it as available. Actual edition/version/plan must decide deployment capabilities.
- The production RBAC example access check tests active identity and grant existence but does not include grant expiry, permission level or tenant bounds. It is not a complete JITPOMI authorization check.
- The `put` reference qualifies the tutorial’s broad idempotency description with concrete concurrency caveats.
- The tutorial demonstrates destructive identity/grant changes for learning. Those snippets are not migration instructions for our real databases.

## Next implementation sequence

1. Write and test the identity/organization/product/membership/entitlement/grant schema using stable IDs. Keep roles contextual and tenant ownership explicit.
2. Add versioned migrations with preflight validation, protected exports and rollback/restore procedures. Preserve existing identities, memberships and record ownership.
3. Introduce typed query inputs and compile/run representative queries against our pinned driver and live development server.
4. Implement shared policy functions and negative tests: wrong organization, other-tenant team/project, expired/future grants, suspended memberships, absent entitlements, conflicting permissions, duplicate provisioning and concurrent revocation.
5. Expose account memberships, available apps and project permissions through Business. Require authorization for grant/invitation administration and prevent self-escalation.
6. Connect jitpomi.com’s account/workspace entry and then an established product such as SEYFR. Keep current Cloudflare responsibilities until each replacement passes real integration tests.

Current private records are a narrow, tested boundary using email/domain ownership. They are not yet this full relationship model. Existing IAM functions also need a selected-tenant and expiry audit before use in shared access. This study adds design evidence and a backlog; it does not implement or certify those remaining pieces.
