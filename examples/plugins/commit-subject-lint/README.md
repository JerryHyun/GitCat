# Commit Subject Lint

A **hook** example. After you make a commit through GitCat's commit UI, it looks
at the new commit's subject line and lets Tama react if it is too long.

## What it shows

- A **`commit-created` hook**. `hooks: [{ event, run }]` runs an external
  command when a lifecycle event fires. `commit-created` fires when you commit
  through GitCat's commit UI.
- Reading commit data from **inside the repo**. A hook's command runs with the
  repository as its working directory, but the hook context does **not** fill in
  `{sha}`/`{file}`/etc. — only `{repo}` is available to hooks. So the hook reads
  what it needs with git itself: `git log -1 --format=%s`.
- Driving Tama from a hook via the same `::gitcat.tama` reaction protocol the
  `hello-tama` example documents. Here it prints `::gitcat.tama problem …` when
  the subject is over 72 characters, otherwise `::gitcat.tama ok …`.

It also ships one **on-demand command** (`Lint: Check HEAD commit subject
length`, `context: "repo"`) that runs the same check against `HEAD` from ⌘K, so
you can try it without making a commit.

## Hooks are observers, not gates

A hook is a **fire-and-forget observer**. It runs *alongside* the GitCat
operation and **cannot veto, block, or roll back** anything — it can only look
and (optionally) make Tama react. It also cannot reach a safety-critical Tama
pose (see the reaction allowlist).

### Events you can hook

`repo-opened`, `repo-switched`, `pre-mutation`, `commit-created`, `undo`.

> `post-mutation` is declared in the manifest schema but is **not fired yet**
> (GitCat has no single post-mutation chokepoint). Don't rely on it.

## Install

Settings → **Plugins** → **Install plugin…**, pick this folder's `plugin.json`.
The hook fires automatically on your next commit; the `lint-head` command shows
up live in ⌘K.

## Windows note

The `run` lines are **POSIX-shell** one-liners (`$(…)`, `${#s}`, `[ … ]`). On
Windows the executor shells out through `cmd.exe`, which does **not** understand
this syntax, so assume a POSIX shell (Git-Bash / WSL) is available. Also note
that a value containing a `cmd.exe` metacharacter (`& | < > ^ % ! "`, CR/LF) is
refused fail-closed on Windows — another reason these examples target a POSIX
shell.
