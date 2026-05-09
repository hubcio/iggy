# Contributing to Apache Iggy

## Issue First

Every new PR that introduces new functionality must link to an approved issue.
PRs without one may be closed at maintainer's discretion.

1. Create an issue or comment under existing
2. Wait for maintainer approval (`good-first-issue` label or comment)
    - Maintainer may request for more details or a different approach
3. Then code

## Size Limits

For new contributors we require to keep PRs under 500 lines of code, unless explicitly approved by a maintainer under linked issue.

## High-Risk Areas

These require design discussion in the issue before coding:

- Persistence (segments, indexes, state, crash recovery)
- Protocol (binary format, wire encoding)
- Concurrency (shards, inter-shard)
- Public API (HTTP, SDKs, CLI)
- Connectors

## PR Requirements

### Run It Locally

**If you can't run it, you can't submit it.**

Authors of PRs must run the code locally. "Relying on CI" is not acceptable.

### Single Purpose

One PR = one thing. Bug fix, refactor, feature - separate PRs. Mixed PRs will be closed.

### Quality Checks

For Rust code:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo build
cargo test
cargo machete
cargo sort --workspace
```

For other languages, check the README in `foreign/{language}/` (e.g., `foreign/go/`, `foreign/java/`).

### Typos Checks

We use [typos](https://github.com/crate-ci/typos):

```bash
cargo install typos-cli --locked
typos
typos --write-changes
```

If it's indeed not a typo, you can set an exception in `.typos.toml`.

### Pre-commit Hooks

We use [prek](https://github.com/j178/prek):

```bash
cargo install prek
prek install
```

## Code Style

### Comments: WHY, Not WHAT

```rust
// Bad: Increment counter
counter += 1;

// Good: Offset by 1 because segment IDs are 1-indexed in the wire protocol
counter += 1;
```

Don't comment obvious code. Do explain non-obvious decisions, invariants, and constraints.

### Commit Messages

Format: `type(scope): subject`

**Good examples from this repo:**

```none
fix(server): prevent panic when segment rotates during async persistence
fix(server): chunk vectored writes to avoid exceeding IOV_MAX limit
feat(server): add SegmentedSlab collection
refactor(server): consolidate permissions into metadata crate
chore(integration): remove streaming tests superseded by API-level coverage
```

Keep subject under 72 chars. Use body for details if needed.

## PR Triage Commands

Comment-driven helpers that keep the review queue scannable. The
[`PR Triage`](./.github/workflows/pr-triage.yml) workflow parses PR comments
line-by-line and updates labels or reviewers via the GitHub API. No bot
account is involved.

### Commands

| Command | Allowed by | Effect |
| --- | --- | --- |
| `/request-review @user-or-team` | committer or PR author | Requests review from `@user` or `@org/team` |
| `/ready` | committer or PR author | Sets `S-waiting-on-review`, removes `S-waiting-on-author` |
| `/author` | committer | Sets `S-waiting-on-author`, removes `S-waiting-on-review` |

Commands must appear at the **start of a line**. Multiple commands in one
comment are processed in order; one command per category (reassign, ready,
author) per comment. Multi-command comments are processed top-to-bottom; the
final label state reflects the last `/ready` or `/author` line. For example,
`/ready` followed by `/author` ends in `S-waiting-on-author`.

### Typical flow

1. Author opens PR. CODEOWNERS auto-requests `@apache/iggy-committers`. The
   PR has no triage label yet.
2. A committer reviews. If changes are needed, they comment `/author` so the
   PR drops out of the review queue and into the author's queue.
3. Author pushes fixes, then comments `/ready`. PR re-enters the review
   queue.
4. Either party can comment `/request-review @specific-committer` to
   reroute the PR to someone with relevant context.

Filter the review queue with
`is:open is:pr label:S-waiting-on-review`.

### Lifecycle automation

State labels are also kept in sync automatically based on PR events:

| Event | Effect |
| --- | --- |
| PR opened (non-draft) | Adds `S-waiting-on-review` if no `S-*` label is set |
| Draft marked ready for review | Adds `S-waiting-on-review` if no `S-*` label is set |
| PR converted back to draft | Removes both `S-*` labels |
| PR closed (merged or rejected) | Removes both `S-*` labels |

Reopened PRs are intentionally not auto-labelled - drop a `/ready` or
`/author` comment to put them back into a queue.

### Behaviour and limits

- **Auth gate.** `/request-review` and `/ready` accept the PR author or any
  repo collaborator (committer). `/author` requires committer. Apache org
  membership alone is not sufficient - the gate is repo-scoped to keep
  unrelated podling members out. "Committer" here means a GitHub
  `COLLABORATOR` or `OWNER` on the apache/iggy repo, which corresponds in
  practice to the `@apache/iggy-committers` ASF team.
- **Bots cannot drive commands.** Bot-suffixed accounts (`*[bot]`, e.g.
  `dependabot[bot]`) fall outside both the committer and PR-author gates,
  even when commenting on their own PRs.
- **Silent failures.** The workflow never replies with comments. If a
  command does not visibly take effect, open the `PR Triage` run under the
  repo's Actions tab; the run log says exactly why (insufficient
  permissions, unknown reviewer, API error).
- **Comment edits are ignored.** Editing an existing comment does not
  re-trigger the workflow. Post a new comment with the corrected command.
- **No effect on issues.** The workflow only runs on PR comments.
- **No checkout, no exec.** The workflow only calls the GitHub REST API
  with the default `GITHUB_TOKEN`. PR code is never checked out and never
  executed, so there is no path for fork-supplied content to read or
  exfiltrate the token.

### Examples

Self-service ready-for-review after fixing review feedback:

```text
/ready
```

Reviewer asks the author to address comments:

```text
/author
```

Multi-command (request a specific reviewer and mark ready in one comment):

```text
/request-review @somebody
/ready
```

Inline prose around a command does not match - the command has to start the
line:

```text
please /ready          # NOT matched (command not at line start)
/ready                 # matched
```

## Close Policy

PRs may be closed if:

- Maintainer feels like proxy between maintainer and LLM
- No approved issue or no approval from a maintainer
- Code not ran and tested locally
- Mixed purposes or purposes not clear
- Can't answer questions about the change
- Inactivity for longer than 7 days

## Questions?

[Discussions](https://github.com/apache/iggy/discussions) or [Discord](https://discord.gg/apache-iggy)

<!-- triage retest 2026-05-09 -->
