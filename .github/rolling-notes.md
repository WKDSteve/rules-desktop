**This is not a release.** It is the newest state of `main`, rebuilt after every green CI run and replaced each time. Nothing here is signed or tested by hand; use a release for anything that matters. The program reads the rule set and can make, change, retire, reinstate and move rules, with Undo and a conflict center; a rule set with no class on the device is never written.

**Windows** (x86-64), **Linux** (x86-64) and **macOS** (Apple silicon, experimental: built and installed by the CI, never started on a real Mac). The files of this page are plain downloads: no GitHub login and no token.

**Linux and macOS:** install for the current user (no root; the script picks the archive of your system, checks it against `SHA256SUMS` and runs `rules-hub install`):

```
curl -fsSL https://github.com/WKDSteve/rules-desktop/releases/download/latest/install.sh | sh
```

Run it again to update. `uninstall.sh` removes it again. The archives are `rules-hub-x86_64-unknown-linux-gnu.tar.gz` and `rules-hub-aarch64-apple-darwin.tar.gz` (the program and its LICENSE); on macOS an archive downloaded by a browser is quarantined, the script's own download is not.

**Windows: install it with the script** (for the current user, no administrator rights, and no SmartScreen stop: the script downloads the program itself and checks it against `SHA256SUMS`). In PowerShell (not elevated):

```
irm https://github.com/WKDSteve/rules-desktop/releases/download/latest/install.ps1 -OutFile install.ps1
.\install.ps1
```

Run it again to update to the newest rolling build. If you save `install.ps1` from this page with the browser instead, run `Unblock-File .\install.ps1` first: the browser marks the file as downloaded, and PowerShell refuses such an unsigned script ("is not digitally signed").

Or take the file yourself: `rules-hub-windows-x86_64.exe`. A double click starts the hub in the background and opens the browser (a second click finds the running hub). Windows SmartScreen warns about an unsigned program: "More info", then "Run anyway". `uninstall.ps1` removes it again; `SHA256SUMS` holds the checksums of all files.

The program needs **Neovim 0.12 or newer**, checkouts of `rules.nvim` and `lib.nvim` (the engine; found through `RULES_ENGINE_DIR` and `LIB_NVIM_DIR`, `REPOS_DIR`, or the settings file) and your rules vault (`RULES_VAULT`). Without them the page says what is missing. `rules-hub --version` names the commit this file was built from; `rules-hub stop` ends the hub. See `docs/USAGE.md` and `docs/WORKFLOW.md`.
