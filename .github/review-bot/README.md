# Skill review bot

A repository committer comments `/skill <name>` on a pull request. A headless agent runs that skill against the pull request head, and the findings arrive as one pull request review with inline comments.

## Commands

- `/skill <name>` runs the skill on the default branch at `.claude/skills/<name>/SKILL.md`, or `.agents/skills/<name>/SKILL.md`.
- `/skill` answers with the list of available skill names.

Any committer can write a skill and use it this way. The gate is a repository write permission, so the PR author alone cannot start a run.

The run reads the skill file, so a skill marked `disable-model-invocation: true` still works here. That flag keeps a skill out of an agent's own reach, which is the point of a human typing the command.

## The two stages

1. `pr-skill-review-run.yml` runs on `issue_comment`. First it parses the command and gates the author. It then makes sure that the skill exists on the default branch. After that it installs the toolchain, checks out the pull request head, and runs the agent. The agent writes `findings.json`.
2. `pr-skill-review-post.yml` runs on `workflow_run`, where the token can write. It reads the trigger comment back from the API, so a rewritten artifact cannot aim the review elsewhere. Then it anchors each finding on a changed line and posts one review. The body opens with the verdict, the reason, and a count per severity. A finding with no anchor goes into the body as text. When the head moved during the run, the whole set goes to the body. This job answers every conclusion, a cancelled run included.

## Toolchain inside a run

- Rust and cargo-nextest, through the repository action `.github/actions/utils/setup-rust-with-cache`.
- serena (`github.com/oraios/serena`), served to the agent as an MCP server for symbols and references. It also supplies the session system prompt, through its own override command, and its session-start and grep-reminder hooks. rust-analyzer comes from the Rust toolchain. An MCP tool list alone does not stop an agent reaching for grep and sed on code. The install pins `serena-agent==1.7.0` from PyPI, so the reviewer cannot change without a commit here.
- rtk (`github.com/rtk-ai/rtk`), newest release, wired in as a `PreToolUse` hook to trim tool output.
- The Claude Code CLI, at the version in `CLAUDE_CODE_VERSION`, pointed at the DeepSeek Anthropic-compatible endpoint.
- The Simple English plugin (`github.com/AminBlg/SimpleEnglish`) at the commit in `SIMPLE_ENGLISH_SHA`, which governs the wording of the posted comments.

## Tests

The agent reads the code rather than running tests. What CI did on the commit reaches it as a file, because it holds no GitHub credentials. A lane that already passed is not worth repeating. When a test is the only way to settle a finding, it builds the binaries first. Those binaries are what the tests under `core/integration` launch, and nothing else builds them.

## Watching a run

The agent step streams each tool call into the job log, one line per call, so an in-progress run is readable in the Actions UI. The raw event stream and the CLI error output sit in the `review-out` artifact next to `findings.json`.

The job log closes with the cost of the run. The line prices the token counts at DeepSeek rates and at two Claude tiers, next to the estimate the CLI itself reports.

Nothing appears on the pull request while a run works. An `issue_comment` run of a fork receives a read-only token, so the first workflow cannot post. The answer arrives with the review at the end.

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
- The poster reads the trigger comment, its author and its pull request from the API. The run snapshots the control file into its own artifact before the agent step starts. The poster answers on the pull request that snapshot names. The agent step runs pull request code as the same user, so it can rewrite the working copy or make it read-only. Neither reaches the snapshot, because the snapshot left the runner first.
- `.claude`, `.agents` and `.serena` are deleted from the pull request checkout, and the default branch copy is restored where one exists. The default branch here carries no `.serena`, so for that path the delete is the half that matters and the restore prints a warning.
- A pull request therefore cannot redefine the skill that reviews it, nor leave a `.serena/project.yml` that serena renders into the agent's own orders. The same holds for the instruction files, `AGENTS.md` and `CLAUDE.md` at any depth, and for `.mcp.json`.
- The agent runs with `--setting-sources user` and `--strict-mcp-config`. Project settings and any MCP server outside the serena one are therefore not loaded at all. A pull request cannot hand the agent new orders or a new tool server.
- The agent runs with `--dangerously-skip-permissions` on an ephemeral runner, which is what an unattended review needs. The run holds nothing worth stealing except the DeepSeek key.
