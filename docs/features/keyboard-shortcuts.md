---
description: "Vim-style keyboard shortcuts and a command palette for the RustMail web UI: move through the inbox, search, filter, open details and delete without the mouse."
---

# Keyboard Shortcuts

RustMail's web UI supports vim-style keyboard navigation.

## Global

These work anywhere in the UI.

| Key | Action |
|-----|--------|
| `Cmd`/`Ctrl`+`K` | Open the [command palette](#command-palette) |
| `j` | Select next message |
| `k` | Select previous message |
| `d` | Delete selected message |
| `u` | Bring back the last deleted message |
| `D` | Delete all messages |
| `s` | Star or unstar the selected message |
| `/` | Focus search bar |
| `i` | Collapse or expand the details rail; on windows under 1300px wide, open or close the details drawer |
| `Esc` | Close the details drawer first; otherwise clear active filters, or close the message detail panel |

`d` leaves the message out of the list straight away, but waits a few seconds
before deleting it, so `u`, or the **Undo** button on the notice that appears,
brings it back. Deleting another message closes the previous one's window, and
so does the window running out; after that the message is gone for good. `D`
asks for confirmation instead, because clearing the inbox cannot be undone.

## Command palette

`Cmd`+`K` on macOS, `Ctrl`+`K` elsewhere, opens the command palette. It works
from anywhere, the search box included; press the chord again, `Esc`, or click
outside to close it. Focus returns to wherever it was before.

Type to narrow the list. Matching is fuzzy: `dk th` finds **Theme: Dark**, and
the words can come in any order. `↑` and `↓` move through the results, `Enter`
runs the highlighted one.

| Group | Commands |
|-------|----------|
| Messages | Typing also searches the loaded messages by subject and sender; picking one selects it |
| Selected message | Star or unstar, mark as read or unread, delete, download .eml, release, show or hide details |
| Filters | Filter by starred, unread, attachments or a tag, remove a filter, clear filters |
| Inbox | Undo delete, clear inbox |
| Appearance | Theme, palette, Rusted, preview width |
| Help | Open settings, show keyboard shortcuts |

The palette lists only commands that do something right now: the message
commands need a selected message, **Release message** needs a configured
relay, **Undo delete** needs a deletion still inside its undo window, and the
theme, palette and preview width already in use are left out. Commands that
also have a shortcut show its key next to them.

Each command runs exactly what its button or key runs. **Delete message**
can be undone with `u`; **Clear inbox** and **Release message** open the same
confirmation as their buttons and never act without it.

While the palette is open, the shortcuts above are off, so typing `d` or `j`
goes into the search box. Keys pressed while focus is inside the HTML preview
stay with the preview, which is sandboxed from the page: click outside it
first.

## Message list

`Tab` reaches the message list as a single stop; the arrow keys then move
within it. Individual rows are not tab stops, so the list stays navigable no
matter how many rows are on screen.

While the list has keyboard focus, the selected row carries a rounded accent
outline; with no message selected, the outline goes around the whole list.
Clicking a row selects it without the outline, so the tinted background alone
marks the selection when you use the mouse.

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

Under the **Unread** filter the selected message stays in the list even after
it has been marked read. Selecting a message marks it read, so without that
exception the row you just landed on would vanish and take your place in the
list with it. The list shrinks behind you as you move instead. Other filters
make no such exception.

Shortcuts are active when no input field is focused, and are ignored when a
modifier key is held, so browser chords such as `Cmd`/`Ctrl`+`D` keep their
normal behaviour; `Cmd`/`Ctrl`+`K` is taken by the command palette. The one
exception is `Esc` inside the details drawer: it closes the drawer even from
the tag input, and focus goes back to whatever opened it.
