The hub as a program for Windows, Linux and macOS, with the interface inside it. The program reads the rule set and can make, change, retire, reinstate and move rules, with Undo and a conflict center; a rule set with no class on the device is never written.

## Install

Without administrator rights, for one user. The `install.ps1` of this release installs exactly this release: it
downloads the archive, checks it against `SHA256SUMS` and hands over to `rules-hub install`. No GitHub login
or token is needed: download it with the command below and run it in your own terminal (PowerShell not elevated).
If you save it from this page with the browser instead, run `Unblock-File .\install.ps1` first: the browser marks the
file as downloaded, and PowerShell refuses such an unsigned script ("is not digitally signed").

```
irm https://github.com/WKDSteve/rules-desktop/releases/download/{tag}/install.ps1 -OutFile install.ps1
.\install.ps1
```

To add a link, pass the argument: `.\install.ps1 -InstallArgs '--shortcut'`. The Help menu of the page registers the hub with your login, if you want that.
The program downloaded by the script is not stopped by SmartScreen; what is checked instead is that it is the file
`SHA256SUMS` of this release lists.

Run the same script again to repair this release (to update, take the script of a newer release); remove the program
with `uninstall.ps1` (your settings stay unless you add `-PurgeSettings`; your vault is never touched). The install
script comes from the release-kit of appkit, the uninstall script from `scripts/` at this tag; both are listed in
`SHA256SUMS`.

## Install on Linux and macOS

For the current user, without root; the script takes the archive of your system, checks it against
`SHA256SUMS` and hands over to `rules-hub install`:

```
curl -fsSL https://github.com/WKDSteve/rules-desktop/releases/download/{tag}/install.sh | sh
```

`uninstall.sh` of this release removes it again. The macOS build (Apple silicon) is experimental: built,
installed and uninstalled by the CI, not started on a real Mac; when its build failed, this release has no macOS file.

## Files

| System | Archive |
| ------ | ------- |
| Windows (x86-64) | `rules-hub-<version>-x86_64-pc-windows-msvc.zip` |
| Linux (x86-64) | `rules-hub-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS (Apple silicon, experimental) | `rules-hub-<version>-aarch64-apple-darwin.tar.gz` |

`SHA256SUMS` lists the checksum of every file (archive and scripts):

```
sha256sum -c SHA256SUMS --ignore-missing
```

The program is not signed yet: Windows may show SmartScreen. The script does not need that.

The rules engine (`rules.nvim`, Neovim 0.12 or newer) is not part of the archive.
