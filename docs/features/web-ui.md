# Web UI

The web UI is served on the HTTP port (`8025` by default) and updates live over the [WebSocket](/features/websocket): new mail shows up the moment it arrives.

## Layout

On a window at least 1300px wide the UI has three panes and a status bar:

| Pane | Contents |
|------|----------|
| Inbox | Search, filter chips (**Starred**, **Unread**, **Attachments**, **Tags**), and the message list |
| Message | Subject, sender and recipients, the star, download and delete actions, and the body |
| Details rail | Summary, authentication, attachments, links and headers of the selected message |
| Status bar | Connection, SMTP address, counts, server version and shortcut hints |

The message pane switches between **Preview** (the rendered [HTML body](/features/html-preview)), **Text** and **Raw**.

Below 1300px the details rail becomes a drawer over the message body. Open it with the **Details** button or `i`, and close it with `i` or `Esc`. See [Keyboard Shortcuts](/features/keyboard-shortcuts).

## Theme and palettes

The gear icon in the header opens **Settings**. Under **Appearance**:

- **Theme**: **Dark**, **Light**, or **System** (the default, which follows the operating system).
- **Palette**: **RustMail** (the default), **Ember**, **Copper & Teal**, **Dawn**, or **Classic**. Each palette tints the neutrals and the accent colour, in both themes.

Both choices are kept in the browser's local storage, so they apply per browser rather than per server.

## Details rail

| Section | Contents |
|---------|----------|
| Summary | From, To, date, size, and the message's tags, which you can add and remove here |
| Authentication | DKIM, SPF and DMARC results (see [Email Authentication](/features/email-auth)) |
| Attachments | Each attachment with its size, as a download link |
| Links | Every link in the message (see below) |
| Headers | **Show headers** lists every header the message carries, as received |

### Links

The **Links** section lists every distinct `http`, `https`, `mailto` and `tel` link in the message, in the order they first appear. Links come from the HTML body's anchors and from URLs in the text body. Other schemes, such as `javascript:` or `file:`, are left out.

Each row shows the host and path, the anchor text, and `×N` when the same link appears more than once. Clicking a row opens the link in a new tab; the copy button next to it copies the full URL. The first 8 links are shown, with **Show all** for the rest.

Two badges flag links worth a second look before the mail reaches real users:

| Badge | Meaning |
|-------|---------|
| `HTTP` | Plain `http://`, which mail clients may warn about or block |
| `LOCAL` | Points at loopback (`localhost`, `127.x.x.x`, `[::1]`) or a `.localhost`, `.local` or `.test` name, which recipients cannot reach |

## Status bar

From left to right:

- **Live**, **Connecting** or **Reconnecting**: the state of the WebSocket connection.
- **SMTP** `host:port`: where to send mail, with a copy button. The port comes from [`GET /api/v1/info`](/api/getInfo), so behind Docker or a proxy that remaps ports, the port you reach from outside may differ.
- The message count and how many are unread.
- The server version.
- Hints for the most used shortcuts.
