# Skill review bot

A repository committer comments `/skill <name>` on a pull request. A headless agent runs that skill against the pull request head, and the findings arrive as one pull request review with inline comments.

## Commands

- `/skill <name>` runs the skill on the default branch at `.claude/skills/<name>/SKILL.md`, or `.agents/skills/<name>/SKILL.md`.
- `/skill` answers with the list of available skill names.

Any committer can write a skill and use it this way. The gate is a repository write permission, so the PR author alone cannot start a run.

The run reads the skill file, so a skill marked `disable-model-invocation: true` still works here. That flag keeps a skill out of an agent's own reach, which is the point of a human typing the command.

## The two stages

1. `pr-skill-review-run.yml` runs on `issue_comment`. First it parses the command and gates the author. It then makes sure that the skill exists on the default branch and posts a start note. After that it installs the toolchain, checks out the pull request head, and runs the agent. The agent writes `findings.json`.
2. `pr-skill-review-post.yml` runs on `workflow_run`, where the token can write. It makes sure that each finding lands on a line of the diff, then posts one review.

## Toolchain inside a run

- Rust and cargo-nextest, through the repository action `.github/actions/utils/setup-rust-with-cache`.
- serena (`github.com/oraios/serena`), served to the agent as an MCP server for symbols and references. rust-analyzer comes from the Rust toolchain.
- rtk (`github.com/rtk-ai/rtk`), newest release, wired in as a `PreToolUse` hook to trim tool output.
- The Claude Code CLI, pointed at the DeepSeek Anthropic-compatible endpoint.
- The Simple English plugin (`github.com/AminBlg/SimpleEnglish`), which governs the wording of the posted comments.

## Files

- `prompt.md` - the task and the output contract for the agent.
- `comment-style.md` - how each posted comment must read.

Both files come from the default branch, never from the pull request under review.

## Secret

`DEEPSEEK_AUTH_TOKEN` holds the DeepSeek key. Add it under repository settings, or with `gh secret set DEEPSEEK_AUTH_TOKEN`.

## Cost and duration

A run installs a toolchain and builds the workspace. Expect ten to thirty minutes of wall clock, and one hosted runner for up to 150 minutes. The agent tokens come from the DeepSeek key.

## Security notes

- Pull request code runs in this workflow, with the DeepSeek key in the environment of the agent step. That is the reason for the committer gate. Keep the key scoped to this bot. If an untrusted person ever drives a run, rotate the key.
- No GitHub token reaches the agent step. The checkout sets `persist-credentials: false` and the step receives no token, so the agent has no way to post or to push. No later step needs a token either.
- `.claude` and `.agents` are restored from the default branch after the pull request checkout, so a pull request cannot redefine the skill that reviews it. The repository `CLAUDE.md` still comes from the pull request, and a committer gate is the control for that.
- The agent runs with `--dangerously-skip-permissions` on an ephemeral runner, which is what an unattended review needs. The run holds nothing worth stealing except the DeepSeek key.
