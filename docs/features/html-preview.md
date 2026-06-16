# HTML Preview

The message detail panel renders the email's HTML body so you see it the way a real client would — including remote images, which load automatically.

## Rendering model

The HTML body is rendered inside a **sandboxed iframe** (`sandbox=""`) with a strict `Content-Security-Policy`. This lets RustMail display untrusted email markup without executing any of it:

| Resource | Policy | Effect |
|----------|--------|--------|
| Scripts | `script-src 'none'` | `<script>`, inline handlers, `javascript:` URLs never run |
| Images | `img-src` self + `data:` + `http:` + `https:` | Inline (`cid:`) and remote images load automatically |
| Styles | `style-src 'unsafe-inline'` | Inline CSS renders; remote stylesheets blocked |
| Fonts / objects / frames | `'none'` | No remote fonts, plugins, or nested frames |
| Forms | `form-action 'none'` | Form submissions are blocked |

Because the iframe has no `allow-scripts` or `allow-same-origin`, email content runs in an opaque origin: it cannot read RustMail's cookies, storage, or DOM, and cannot navigate the parent page.

## Remote images

Remote images load automatically — RustMail is a development mail catcher, so the "sender" is your own application and the priority is showing the message exactly as sent, with no extra click. Inline images referenced by `cid:` are served from the message's attachments.

Requests for remote content are sent with `Referrer-Policy: no-referrer`, so the originating URL is not leaked.

::: warning Exposed instances
Loading remote content means opening a message triggers an outbound request to whatever URLs it references. On the default loopback bind (`127.0.0.1`) this is harmless. If you bind RustMail to a non-loopback address (`RUSTMAIL_BIND`) and let it receive untrusted mail, be aware that viewing a message will fetch remote content from the viewer's network. The API is unauthenticated by design — keep exposed instances behind your own access controls.
:::
