# Security Policy

Thank you for helping keep GitCat and its users safe.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues, discussions, or pull requests.**

Report privately through GitHub's built-in flow:

1. Go to the [**Security** tab](https://github.com/zangjiucheng/GitCat/security) of this repository.
2. Click **Report a vulnerability** to open a private advisory (this is only visible to you and the maintainer).

If you can't use GitHub's private reporting, email **git.jiucheng@gmail.com** with the subject line `GitCat security`.

Please include:

- The GitCat version (Help → About, or the app window title).
- Your OS and architecture (e.g. macOS 14 arm64, Windows 11 x64, Ubuntu 24.04 x64).
- A description of the issue and its impact.
- Step-by-step reproduction, and a proof of concept if you have one.

## What to expect

- **Acknowledgement** within 5 days.
- An initial **assessment** within 10 days, and regular updates while we work on a fix.
- Once a fix ships, we'll credit you in the release notes and the advisory unless you'd prefer to stay anonymous.

Please give us a reasonable window to release a fix before any public disclosure. We aim to resolve confirmed issues within 90 days.

## Scope

GitCat is a local desktop Git client. It runs Git operations against repositories you already have on disk and does **not** connect to any AI service or send your code anywhere. Areas of particular interest:

- Command/argument injection through crafted repository data (branch/tag/remote names, `.gitmodules`, submodule URLs, config values, file paths).
- Path traversal or writes outside the selected working tree.
- Anything that lets untrusted repository content execute code or escape the WebView (Tauri IPC surface, the Content-Security-Policy).
- Mishandling of credentials or the WSL command-routing path.

## Supported versions

GitCat is pre-1.0; security fixes land in the latest release only. Please make sure you're on the newest version before reporting.

| Version | Supported |
| ------- | --------- |
| latest `0.9.x` | ✅ |
| older | ❌ |

> Tip: enable **Private vulnerability reporting** under *Settings → Code security and analysis* so the "Report a vulnerability" button above is available to everyone.
