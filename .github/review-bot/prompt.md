# Skill review task

You review pull request {{REPO}}#{{PR_NUMBER}}. A repository skill says what to look for. The rules below say how to report it.

## What is on disk

- The working directory is a checkout of the pull request head.
- `{{RUNNER_TEMP}}/pr.diff` is the diff of the pull request against its merge base with `{{BASE_BRANCH}}`.
- `{{RUNNER_TEMP}}/pr-stat.txt` and `{{RUNNER_TEMP}}/pr-files.txt` describe the same diff.
- Rust, cargo-nextest and the serena MCP server are installed. Use serena for definitions, references and symbols, and ripgrep for text.
- `.claude` and `.agents` come from the base branch, not from the pull request.

## What to do

1. Read `.claude/skills/{{SKILL}}/SKILL.md`, or `.agents/skills/{{SKILL}}/SKILL.md`, and follow it against this diff.
2. Read the code before you write a finding. Open the caller, the definition and the configuration. When a build, a clippy run or a test decides the question, run it.
3. When the skill is done, write `{{RUNNER_TEMP}}/findings.json`.

## What to report

Report what the diff changes. A pre-existing problem that the diff makes worse, or that the new code depends on, is in scope. An unrelated old problem is not.

Every finding must be provable from this checkout. Make sure that the symbol, the call site and the configuration support it, then write it down. Drop what you cannot prove. A short review of proven findings is worth more than a long review of guesses.

## The findings file

`{{RUNNER_TEMP}}/findings.json` is the only deliverable:

```json
{
  "summary": "one or two sentences for the review body, or an empty string",
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
