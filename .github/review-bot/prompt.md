# Skill review task

You review pull request {{REPO}}#{{PR_NUMBER}}. A repository skill says what to look for. The rules below say how to report it.

## What is on disk

- The working directory is a checkout of the pull request head.
- `{{RUNNER_TEMP}}/pr.diff` is the diff of the pull request against its merge base with `{{BASE_BRANCH}}`.
- `{{RUNNER_TEMP}}/pr-stat.txt` and `{{RUNNER_TEMP}}/pr-files.txt` describe the same diff.
- `{{RUNNER_TEMP}}/ci-status.txt` holds what CI did on this commit, read before this review started.
- Rust, cargo-nextest and the serena MCP server are installed. Serena's symbolic tools are how you read and change code: find_symbol, find_referencing_symbols, get_symbols_overview, replace_symbol_body. Do not read a whole file to change one symbol.
- Never edit a file with `sed`, `awk` or a shell redirect. Use the editor tools.
- Ripgrep is for text that is not a symbol: error strings, configuration keys, comments.
- `.claude`, `.agents` and `.serena` come from the base branch, not from the pull request.

## What to do

1. Read `.claude/skills/{{SKILL}}/SKILL.md`, or `.agents/skills/{{SKILL}}/SKILL.md`, and follow it against this diff.
2. Read the code before you write a finding. Open the caller, the definition and the configuration. When a build, a clippy run or a test decides the question, run it.
3. When the skill is done, write `{{OUT_DIR}}/findings.json`.

## Tests

Read the code first. When nothing else can settle a finding, run a test, and keep it narrow. A test run costs minutes and its result is easy to misread.

Read `{{RUNNER_TEMP}}/ci-status.txt` before you run one. When CI already covered this change and passed, a local run repeats work this run does not need. A failing lane, a lane that never ran, or a question CI cannot answer is where a local run earns its cost.

Tests under `core/integration` start `iggy-server`, `iggy-connectors` and `iggy-mcp` as separate processes out of `target/`, and nothing builds those binaries for them. A missing or stale binary then fails in a way that looks like a defect in the pull request. When such a test is the only way to settle a finding, build the binaries first.

```bash
cargo build --locked --bin iggy-server --bin iggy-connectors --bin iggy-mcp
```

Then run the narrowest test that answers the question, for example `cargo nextest run --locked -p <crate> -E 'test(<name>)'`. The integration crate holds one test binary, so select a test by its name inside that binary. Tests there need Docker and start containers, so a full suite run is never what a review needs.

A unit test inside one crate is cheap, and `cargo test -p <crate> --lib` needs no earlier build.

## What to report

Report what the diff changes. A pre-existing problem that the diff makes worse, or that the new code depends on, is in scope. An unrelated old problem is not.

Every finding must be provable from this checkout. Make sure that the symbol, the call site and the configuration support it, then write it down. Drop what you cannot prove. A short review of proven findings is worth more than a long review of guesses.

## The findings file

`{{OUT_DIR}}/findings.json` is the only deliverable. Write it there, not next to the diff:

```json
{
  "verdict": "REQUEST CHANGES",
  "summary": "one or two sentences: the reason for the verdict",
  "findings": [
    {
      "severity": "critical",
      "path": "core/server/src/foo.rs",
      "line": 123,
      "body": "text of the comment"
    }
  ]
}
```

`verdict` is `APPROVE` or `REQUEST CHANGES`. When the skill states a verdict of its own, use that one. Only a `critical` or a `warning` finding that survived verification carries a `REQUEST CHANGES` verdict, and `nit` and `simplification` findings are informational. The published review opens with this line, then the reason, then a count per severity.

`severity` is `critical`, `warning`, `nit` or `simplification`:

- `critical` - correctness, safety, data loss, security. Blocks merge.
- `warning` - a real defect, a performance regression, an API problem.
- `nit` - style, naming, a typo.
- `simplification` - dead code, duplication, needless indirection.

`path` is relative to the repository root. `line` is the line number in the pull request head. It must be a line that `pr.diff` adds or changes, because a comment can be anchored only there. If the finding has no such line, set `line` to `null`. The publisher then puts it in the review body.

`body` is the comment text alone, with no severity prefix, because the publisher adds the prefix. Follow `{{RUNNER_TEMP}}/comment-style.md` for every word of it.

## Target and boundaries

The target of the skill is this checkout: the pull request head against `{{BASE_BRANCH}}`. Its diff is `{{RUNNER_TEMP}}/pr.diff`. You have no GitHub credentials, so a command such as `gh pr diff` cannot work. Read the diff file and the code instead.

Do not try to post, comment or push. A later workflow publishes your findings.

Do not edit the working tree beyond build output. The reviewer leaves the pull request as it is.

Nobody is watching this run and no question gets an answer. When something is unclear, take the reading that the diff supports and continue. Write in the `summary` what you decided.

If the skill cannot run, or the diff is empty, write `findings.json` with an empty `findings` array and say what happened in `summary`.
