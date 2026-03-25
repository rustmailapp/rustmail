# Neovim Plugin

[rustmail.nvim](https://github.com/rustmailapp/rustmail.nvim) is a Neovim client that connects to a running RustMail instance, providing a floating-window UI for browsing captured emails. New messages appear automatically via background polling.

## Requirements

- Neovim >= 0.10
- `curl` on PATH
- `rustmail` binary on PATH (or configure a custom path)

## Installation

::: code-group

```lua [lazy.nvim]
{
  "rustmailapp/rustmail.nvim",
  cmd = "RustMail",
  opts = {},
}
```

```lua [packer.nvim]
use {
  "rustmailapp/rustmail.nvim",
  config = function()
    require("rustmail").setup()
  end,
}
```

:::

## Configuration

Pass options to `require("rustmail").setup()`:

```lua
require("rustmail").setup({
  host = "127.0.0.1",        -- RustMail server host
  port = 8025,               -- RustMail HTTP port
  smtp_port = 1025,          -- RustMail SMTP port (used by auto_start)
  auto_start = false,        -- start RustMail daemon if not already running
  binary = "rustmail",       -- path to rustmail binary
  poll_interval = 2000,      -- polling interval in ms
  float = {
    width = 0.8,             -- fraction of editor width
    height = 0.8,            -- fraction of editor height
    border = "rounded",      -- border style
  },
  keymaps = {
    list = {
      open = "<CR>",
      delete = "dd",
      toggle_read = "mr",
      toggle_star = "ms",
      refresh = "R",
      search = "/",
      quit = "q",
      clear_all = "D",
    },
    detail = {
      back = "<BS>",
      delete = "dd",
      toggle_read = "mr",
      toggle_star = "ms",
      quit = "q",
      view_raw = "gR",
      view_attachments = "ga",
      view_auth = "gA",
    },
  },
})
```

### Auto-Start

When `auto_start = true`, the plugin performs a health check on startup. If RustMail is not reachable, it spawns a detached `rustmail serve` process with the configured ports. Stop it with `:RustMail stop`.

## Commands

| Command | Description |
|---|---|
| `:RustMail` | Open the message list (default) |
| `:RustMail toggle` | Toggle the message list |
| `:RustMail close` | Close all windows and stop polling |
| `:RustMail stop` | Stop the auto-started daemon |

## Keymaps

All keymaps are buffer-local to RustMail windows and fully configurable via the `keymaps` option.

::: info
`dd` shadows Vim's built-in delete-line in RustMail buffers.
:::

### Message List

| Key | Action |
|---|---|
| `<CR>` | Open selected message |
| `dd` | Delete selected message |
| `mr` | Toggle read/unread |
| `ms` | Toggle starred |
| `R` | Refresh list |
| `/` | Search messages (empty input clears search) |
| `D` | Delete ALL messages (with confirmation) |
| `q` | Close |

### Message Detail

| Key | Action |
|---|---|
| `<BS>` | Back to list |
| `dd` | Delete message |
| `mr` | Toggle read/unread |
| `ms` | Toggle starred |
| `gR` | View raw message (RFC 5322) |
| `ga` | View attachments |
| `gA` | View authentication results (DKIM/SPF/DMARC) |
| `q` | Close |

## Lua API

```lua
local rustmail = require("rustmail")

rustmail.setup({ ... })      -- configure the plugin
rustmail.open()              -- open the message list
rustmail.close()             -- close all windows, stop polling
rustmail.toggle()            -- toggle the message list
rustmail.ensure_daemon()     -- start rustmail if not running
rustmail.stop_daemon()       -- stop auto-started daemon
```
