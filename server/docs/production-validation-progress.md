# Business deployment validation — 2026-10-08

Status: **the agreed bounded deployment-validation suite passed, including a second capacity run after restart**. Validation uses the existing free TypeDB Cloud `business_dev` database. Production data is not part of these tests.

## Earlier evidence

- A live connection-loss/transaction-timeout test passed (11.89 seconds). Buffered changes remained absent after timeout, disconnect before commit, and the observed commit/disconnect race; a fresh connection could write afterward. The race did not demonstrate a committed transaction with a lost acknowledgement.
- A backup/restore drill exported `business_dev`, restored it into a temporary database on the same cluster, compared 985 selected identity, relationship and policy rows, and removed the restored database. It passed in 113.83 seconds. This snapshot preceded the grant-function split below.
- Render's new-service form uses the Free plan ($0, 0.1 CPU, 512 MB). With explicit approval, ten environment variables were imported from a protected local file, including non-admin TypeDB credentials and a fresh JWT signing secret. The tested commit `9b1e169` is now deployed at https://jitpomi-business-validation.onrender.com, with automatic deployments disabled. The TypeDB runtime account is not limited by the provider to this database.

## Performance investigation

The provisional acceptance workload is ten active tenants, ten concurrent users, five minutes of mixed protected reads/writes. The test uses the real TypeDB Cloud database through the in-process HTTP router. It requires at least 1,000 requests, p95 <= 5 seconds, p99 <= 10 seconds and no unexpected status codes. This is not a public-network load test.

Earlier attempts failed with 408 responses. Bounding active requests at two and waiting requests at 32 prevents unbounded demand, but by itself yielded 503 overload responses and did not fix slow queries. No threshold was relaxed to turn those failures into passes.

Grant evaluation originally combined user and team grantees in one disjunction. A sequential diagnostic timed out after ten seconds on a grant check. A later idle diagnostic completed the full authorization check in 8,537 ms. After splitting direct and team grants into separate functions and successfully migrating the development database, the same diagnostic completed authorization in 302 ms (identity 78 ms; membership 85 ms; entitlement 73 ms; allow 272 ms; deny 64 ms). These are individual samples, not a capacity certification or proof that all latency came from that expression. The first migration attempt timed out opening a schema transaction; the retry succeeded.

## Earlier regression and deployment results

- Correctness regression after the policy-function changes: **passed**, 13 tests in 101.60 seconds, including hosted auth/revocation, tenant isolation, connection recovery, identity concurrency, invalid migration rejection, protected-write atomicity and workspace policies. Clippy passed with warnings denied.
- Unchanged mixed-workload acceptance test: **passed**. Ten tenants and ten concurrent users completed 230 cycles / 1,610 requests in 307.60 seconds, zero unexpected responses, p95 2,181 ms, p99 2,218 ms, maximum 2,271 ms. This used the in-process router and live TypeDB Cloud, not a deployed HTTP endpoint.
- Deployment completed on Render Free from commit `9b1e169`; release build and HTTPS health check passed.
- Public HTTPS correctness: all three authentication/revocation, tenant isolation and workspace-policy suites passed in 97.66 seconds. Requests use curl with certificate verification enabled, no redirect following, and tokens/body supplied through stdin. Raw queries return 404, missing credentials 401, oversized bodies 400 with a size-limit error.
- A controlled Render restart passed after observing a new process start: saved record, valid session and token revocation persisted (4.03-second verification). The initial attempt overlapped two restart requests and returned 502 during a cleanup DELETE that nevertheless committed; a subsequent GET returned 404. The old session was revoked and a fresh fixture was used for the controlled run. This is evidence of uncertain write outcomes during disruption, not zero-downtime certification.
- Record exact results and distinguish tested recovery from untested provider failover, long network partitions and highly available operation. A free sleeping service does not establish an always-on availability guarantee.

## Earlier public load shortfall

The unchanged five-minute target failed its minimum-request gate: 980 requests
(140 cycles) across ten tenants in 303.05 seconds, against the required 1,000.
There were zero unexpected responses. p95 was 3,493 ms, p99 3,798 ms and maximum
5,343 ms, so the latency gates passed. No threshold was relaxed and this is not
recorded as a full acceptance pass. The client starts curl separately per request;
network latency, connection setup and process overhead are included. This run
does not attribute the shortfall to either Business code or provider resources.
The earlier in-process result remains a separate 1,610-request pass.

The follow-up investigation below measured request stages and used a persistent
HTTPS client without reducing the workload or thresholds.
The free service sleeps when idle and restart disruption was observed; this is not
an always-on or lossless-provider-failover certification.


## Final deployment validation

The deployed executable is commit `dfd90b8`, with `BUSINESS_ACTIVE_REQUESTS=3`.
The same budget is recorded in the deployment manifest and prepared development
configuration. The queue remains bounded at 32 waiting requests and five seconds.
No authentication, revocation or policy checks were removed or cached.

The pooled HTTPS client alone did not fix the two-request shortfall: it again
completed 980 requests. A short timing diagnostic measured admission wait p95
2,742.8 ms and handler p95 718.7 ms. Increasing the active budget to three allowed
more requests to execute concurrently. An initial five-minute attempt at three
failed with six workers seeing HTTP 503 responses. That attempt did not capture
the response origin, so its cause remains unclassified; it is retained in the
JSON evidence. Subsequent test instrumentation classifies server errors without
printing response bodies, tokens or credentials and prints timings even if a
worker fails.

A subsequent unchanged five-minute public HTTPS run passed: 1,470 requests,
210 complete CRUD/denial cycles, 303.93 seconds, zero unexpected responses,
p95 2,492 ms, p99 2,610 ms, maximum 3,343 ms. Admission wait p95 was 1,738.1 ms
and handler p95 701.5 ms. The throughput gain is consistent with the increased
admission budget; it is not evidence of a faster database query implementation.

All 14 final regression tests passed in 144.01 seconds. These include the three
public HTTPS auth/isolation/workspace suites, live database uniqueness/concurrency,
invalid migration rejection, protected-write atomicity, connection recovery, and
an expanded backup/restore drill. The drill compares token revocations, credentials,
entitlements, grants, team membership, project/record ownership, identity sets and
private policy results. Only the temporary restored database is deleted. Exported
credentials are kept in a private temporary directory and removed on completion.

One controlled Render restart was requested. Logs confirmed a new process at
2026-10-08 19:12:36 UTC (provider marker `bxknl`), followed by HTTPS health 200.
The persistence/revocation test passed in 4.07 seconds: record retained, valid token
accepted, revoked token rejected, fixture record cleaned up and remaining session
logged out.

This validates a bounded foundation workload on the free validation deployment,
not always-on service or arbitrary scale. The historical 503 failure remains part
of the reliability evidence. Provider failover, long network partitions, managed
backup retention and global authentication abuse controls are not certified by
these checks. No paid resources were provisioned.


The second full load run, after the confirmed restart, also passed: 1,491 requests,
213 cycles, 306.93 seconds, zero unexpected responses, p95 2,404 ms, p99 2,511 ms,
maximum 2,845 ms. Admission p95 was 1,700.1 ms and handler p95 700.8 ms. Both
successful runs retained the original >=1,000-request and latency thresholds.

Final public edge checks passed using curl with TLS verification enabled:
health 200, missing credentials 401, raw query endpoint 404, oversized body 400
with the size-limit message, and HTTP-to-HTTPS redirect 301. A local Python urllib
attempt could not find its CA trust chain; verification was not disabled to work
around it. The successful load tests used verified TLS through reqwest.

Formatting, Clippy with warnings denied, and the tracked-file credential scan
passed. The deployment manifest, configuration generator and example now record
the tested active budget of three. Later changes in this validation pass affect
tests, documentation and configuration artifacts; the deployed executable remains
`dfd90b8`. This is a completed acceptance pass for the stated scope, not a blanket
production certification. Earlier failed runs are retained above and in the JSON.
