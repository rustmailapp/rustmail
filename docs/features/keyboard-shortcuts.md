# Keyboard Shortcuts

RustMail's web UI supports vim-style keyboard navigation.

## Global

These work anywhere in the UI.

| Key | Action |
|-----|--------|
| `j` | Select next message |
| `k` | Select previous message |
| `d` | Delete selected message |
| `D` | Delete all messages |
| `s` | Star or unstar the selected message |
| `/` | Focus search bar |
| `Esc` | Clear active filters, or close the message detail panel |

## Message list

`Tab` reaches the message list as a single stop; the arrow keys then move
within it. Individual rows are not tab stops, so the list stays navigable no
matter how many rows are on screen.

| Key | Action |
|-----|--------|
| `↓` | Select next message |
| `↑` | Select previous message |
| `Home` | Select the first message |
| `End` | Select the last loaded message |

The inbox loads messages a page at a time, so `End` lands on the last message
fetched so far. Pressing it again after the next page arrives moves further
down.

The star toggle on each row is not a tab stop either; use `s` to star the
selected message.

Shortcuts are active when no input field is focused, and are ignored when a
modifier key is held, so browser chords such as `Cmd`/`Ctrl`+`D` keep their
normal behaviour.
