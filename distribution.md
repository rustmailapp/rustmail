# Distribution Plan

Internal reference for packaging and distribution channels.

## Current Channels

| Channel | Status | Notes |
|---------|--------|-------|
| **Homebrew** | Ready | `brew install rustmailapp/rustmail/rustmail` |
| **Docker** | Ready | `smyile/rustmail:latest` |
| **GitHub Releases** | Ready | Multi-platform binaries via release workflow |
| **Source** | Ready | `make build` (requires Node + pnpm + Rust) |
| **crates.io** | Not used | All crates yanked. Binary can't be published (embedded UI requires Node build step) |

## Homebrew

### Phase 1: Homebrew Tap (when ready to publish)

A tap is a self-hosted formula repository. Users install with:

```sh
brew install rustmailapp/rustmail/rustmail
```

**Setup:**

1. Create repo `rustmailapp/homebrew-rustmail` on GitHub
2. Add `Formula/rustmail.rb` pointing to a tagged release tarball
3. Formula builds from source using `depends_on "rust" => :build` and `depends_on "node" => :build`
4. Optionally use pre-built binaries from GitHub Releases instead of building from source

**Example formula (build from source):**

```ruby
class Rustmail < Formula
  desc "Self-hosted SMTP mail catcher with web UI, REST API, and CI assertions"
  homepage "https://github.com/rustmailapp/rustmail"
  url "https://github.com/rustmailapp/rustmail/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "<sha256 of tarball>"
  license "MIT OR Apache-2.0"

  depends_on "rust" => :build
  depends_on "node" => :build

  def install
    system "corepack", "enable"
    system "pnpm", "--dir", "ui", "install", "--frozen-lockfile"
    system "pnpm", "--dir", "ui", "build"
    system "cargo", "install", "--locked", "--root", prefix, "--path", "crates/rustmail-server"
  end

  test do
    assert_match "rustmail", shell_output("#{bin}/rustmail --version")
  end
end
```

**Tools that can auto-generate formulas from GitHub release assets:**
- `cargo-brew` (https://crates.io/crates/cargo-brew)
- `formulaic` (https://github.com/ceejbot/formulaic)

### Phase 2: homebrew-core (75+ stars)

Submission to the official Homebrew repository requires:

- **>=75 GitHub stars** (or >=30 forks, or >=30 watchers)
- **Project age >30 days**
- **Stable tagged release**
- **Someone other than the author submits the PR** (community member)
- **Builds on latest 3 macOS versions** (ARM + x86_64) + x86_64 Linux
- **No unpatched security vulnerabilities**

Process:
1. Meet thresholds above
2. A community member submits PR to `Homebrew/homebrew-core`
3. `brew test-bot` CI builds bottles (pre-compiled binaries) for all platforms
4. Homebrew maintainers review (typically 1-4 weeks)

Precedent: ripgrep, bat, fd, hyperfine, delta (all Rust CLI tools in homebrew-core). Mailpit (Go, same problem space) is also in homebrew-core.

Reference: https://docs.brew.sh/Acceptable-Formulae

## Future Channels (not planned yet)

- **AUR** (Arch Linux) — PKGBUILD, straightforward for Rust binaries
- **Nix** — nixpkgs PR, similar community process to Homebrew
- **Scoop** (Windows) — JSON manifest in a bucket repo
- **Snap / Flatpak** — lower priority, Docker covers Linux well
