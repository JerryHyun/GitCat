# Install

Download the installer for your platform from the [Releases page](https://github.com/zangjiucheng/GitCat/releases). Every release is built from the same tag across a 6-platform matrix:

| Platform | Architectures | Format |
| --- | --- | --- |
| macOS | Apple Silicon, Intel | `.dmg` |
| Windows | x86_64, arm64 | `.msi` / `.exe` |
| Linux | x86_64, arm64 | `.deb`, `.rpm`, `.AppImage` |

## Unsigned builds

GitCat isn't code-signed or notarized yet, so your OS will warn that it comes from an unidentified developer. That's expected for a pre-1.0 open-source build — the downloads point at the same [GitHub Releases](https://github.com/zangjiucheng/GitCat/releases) the source is built from. Here's how to get past each platform's gate.

### macOS

Because the app is unsigned **and** arrived through a browser, macOS quarantines it.

- **Try first:** right-click (or Control-click) GitCat in Applications → **Open**, then **Open** again in the dialog. You only need to do this once.
- **macOS Sequoia (15) and newer**, where that right-click bypass was removed: launch it once (it'll be blocked), then open **System Settings → Privacy & Security**, scroll to the GitCat message, and click **Open Anyway**.
- **If you see “GitCat is damaged and can’t be opened”** (common on Apple Silicon), clear the quarantine flag in Terminal, then launch normally:

  ```bash
  xattr -dr com.apple.quarantine /Applications/GitCat.app
  ```

### Windows

On the blue **“Windows protected your PC”** SmartScreen prompt, click **More info → Run anyway**. It may reappear after each new download.

### Linux

No OS-level gate. For the `AppImage`, mark it executable first (it needs FUSE, preinstalled on most desktops):

```bash
chmod +x GitCat_*.AppImage
./GitCat_*.AppImage
```

The `.deb` / `.rpm` packages install normally — `sudo dpkg -i GitCat_*.deb` or `sudo rpm -i GitCat-*.rpm`.

> Signed macOS/Windows builds are on the roadmap; until then these one-time steps are the trade-off for an unsigned open-source release.

## Building from source

If you'd rather build it yourself (or want to run a development build), see [Development](https://github.com/zangjiucheng/GitCat#development) in the README — you'll need [Rust](https://www.rust-lang.org/tools/install), [Node](https://nodejs.org) 22+, and [pnpm](https://pnpm.io):

```bash
git clone https://github.com/zangjiucheng/GitCat.git
cd GitCat
pnpm install
pnpm tauri dev
```

Want a repo to poke around in instead of pointing GitCat at something real? `pnpm demo` builds one at `~/gitcat-demo` with branches, tags, a submodule, stashes, a diverged remote, an unmerged branch that conflicts with `main` on purpose, and a bisectable bug.
