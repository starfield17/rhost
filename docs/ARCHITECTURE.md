# Remote Host Adapter — Architecture & Implementation Framework

> Working name: **rhost**  
> Audience: Codex implementing the repository  
> Language recommendation: **Go**  
> Architecture style: disposable CLI + externalized persistence  
> Required product shape: **Skill + source + release binary**, not MCP-first

---

This file is the **map**. The architecture is expanded under
[`docs/architecture/`](architecture/), one file per Part, so a reader — human or
agent — can load only the part a task needs instead of a 1900-line document.

Section numbers `§0`–`§53` are stable. Every `docs/ARCHITECTURE.md §N`
reference in the code, `AGENTS.md`, `README.md` and `skills/rhost/SKILL.md` still resolves:
find §N in the list below to see which file holds it. Each Part file links back
to this map.

## Foundations

- [0. Implementation directive](architecture/00-implementation-directive.md) —
  the invariant every other Part is accountable to.

## [Part I — System shape](architecture/01-system-shape.md)

- [1. Top-level architecture](architecture/01-system-shape.md#1-top-level-architecture)
- [2. Persistence ownership](architecture/01-system-shape.md#2-persistence-ownership)

## [Part II — Repository boundaries](architecture/02-repository-boundaries.md)

- [3. Recommended Go repository layout](architecture/02-repository-boundaries.md#3-recommended-go-repository-layout)
- [4. Suggested core interfaces](architecture/02-repository-boundaries.md#4-suggested-core-interfaces)

## [Part III — Host/configuration model](architecture/03-host-configuration.md)

- [5. OpenSSH config is the authentication source of truth](architecture/03-host-configuration.md#5-openssh-config-is-the-authentication-source-of-truth)
- [6. Host capability discovery](architecture/03-host-configuration.md#6-host-capability-discovery)

## [Part IV — SSH transport](architecture/04-ssh-transport.md)

- [7. v0.1 transport: system OpenSSH](architecture/04-ssh-transport.md#7-v01-transport-system-openssh)
- [8. ControlMaster management](architecture/04-ssh-transport.md#8-controlmaster-management)
- [9. Command construction](architecture/04-ssh-transport.md#9-command-construction)

## [Part V — `exec`](architecture/05-exec.md)

- [10. CLI surface](architecture/05-exec.md#10-cli-surface)
- [11. Foreground timeout](architecture/05-exec.md#11-foreground-timeout)

## [Part VI — `session`](architecture/06-session.md)

- [12. Why tmux is the v0.1 session backend](architecture/06-session.md#12-why-tmux-is-the-v01-session-backend)
- [13. Session identity and remote layout](architecture/06-session.md#13-session-identity-and-remote-layout)
- [14. Session creation](architecture/06-session.md#14-session-creation)
- [15. Session command modes](architecture/06-session.md#15-session-command-modes)
- [16. Reliable input injection](architecture/06-session.md#16-reliable-input-injection)
- [17. Command-boundary protocol](architecture/06-session.md#17-command-boundary-protocol)
- [18. Incremental session reads](architecture/06-session.md#18-incremental-session-reads)
- [19. Human attach](architecture/06-session.md#19-human-attach)

## [Part VII — `job`](architecture/07-job.md)

- [20. Job semantics](architecture/07-job.md#20-job-semantics)
- [21. Remote job state layout](architecture/07-job.md#21-remote-job-state-layout)
- [22. Detached process backend](architecture/07-job.md#22-detached-process-backend)
- [23. Job status](architecture/07-job.md#23-job-status)
- [24. Job log polling](architecture/07-job.md#24-job-log-polling)
- [25. Job signals](architecture/07-job.md#25-job-signals)

## [Part VIII — files](architecture/08-files.md)

- [26. File operations](architecture/08-files.md#26-file-operations)
- [27. v0.1 transfer backend](architecture/08-files.md#27-v01-transfer-backend)
- [28. Safe synchronization](architecture/08-files.md#28-safe-synchronization)

## [Part IX — status and monitor](architecture/09-status-and-monitor.md)

- [29. `status`](architecture/09-status-and-monitor.md#29-status)
- [30. Remote probe design](architecture/09-status-and-monitor.md#30-remote-probe-design)
- [31. `watch`](architecture/09-status-and-monitor.md#31-watch)

## [Part X — structured output](architecture/10-structured-output.md)

- [32. JSON is a public compatibility surface](architecture/10-structured-output.md#32-json-is-a-public-compatibility-surface)
- [33. Error taxonomy](architecture/10-structured-output.md#33-error-taxonomy)

## [Part XI — the skill](architecture/11-skill.md)

- [34. Required Skill behavior](architecture/11-skill.md#34-required-skill-behavior)

## [Part XII — security and audit](architecture/12-security-and-audit.md)

- [35. Security stance](architecture/12-security-and-audit.md#35-security-stance)
- [36. Audit trail](architecture/12-security-and-audit.md#36-audit-trail)

## [Part XIII — Portal source as reference](architecture/13-portal-reference.md)

- [37. What Codex should inspect in local `portal-mcp-server`](architecture/13-portal-reference.md#37-what-codex-should-inspect-in-local-portal-mcp-server)

## [Part XIV — releases and installation](architecture/14-releases-and-installation.md)

- [38. Release artifacts](architecture/14-releases-and-installation.md#38-release-artifacts)
- [39. Version output](architecture/14-releases-and-installation.md#39-version-output)

## [Part XV — testing](architecture/15-testing.md)

- [40. Test layers](architecture/15-testing.md#40-test-layers)
- [41. Required persistence tests](architecture/15-testing.md#41-required-persistence-tests)
- [42. Prove boundary checks work](architecture/15-testing.md#42-prove-boundary-checks-work)

## [Part XVI — staged implementation](architecture/16-milestones.md)

- [43. Milestone 0 — repository skeleton](architecture/16-milestones.md#43-milestone-0--repository-skeleton)
- [44. Milestone 1 — host + doctor + exec](architecture/16-milestones.md#44-milestone-1--host--doctor--exec)
- [45. Milestone 2 — sessions](architecture/16-milestones.md#45-milestone-2--sessions)
- [46. Milestone 3 — jobs](architecture/16-milestones.md#46-milestone-3--jobs)
- [47. Milestone 4 — files](architecture/16-milestones.md#47-milestone-4--files)
- [48. Milestone 5 — status/watch](architecture/16-milestones.md#48-milestone-5--statuswatch)
- [49. Milestone 6 — hardening](architecture/16-milestones.md#49-milestone-6--hardening)

## [Part XVII — deferred architecture](architecture/17-deferred-architecture.md)

- [50. When a local daemon becomes justified](architecture/17-deferred-architecture.md#50-when-a-local-daemon-becomes-justified)
- [51. When a remote runtime becomes justified](architecture/17-deferred-architecture.md#51-when-a-remote-runtime-becomes-justified)

## [Part XVIII — anti-goals](architecture/18-anti-goals.md)

- [52. Things Codex should actively avoid](architecture/18-anti-goals.md#52-things-codex-should-actively-avoid)

## [Part XIX — Definition of done](architecture/19-definition-of-done.md)

- [53. First release](architecture/19-definition-of-done.md#53-first-release)

## [Part XX — Codex self-check](architecture/20-self-check.md)

- [self-check](architecture/20-self-check.md)
