# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

**Please do not open public issues for security vulnerabilities.**

If you discover a security vulnerability in RustMail, please report it responsibly:

1. **Email:** Send a detailed report to the repository maintainers via GitHub private vulnerability reporting
   (Security tab → "Report a vulnerability") or by opening a [security advisory](https://github.com/rustmailapp/rustmail/security/advisories/new).

2. **Include:**
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact assessment
   - Suggested fix (if applicable)

3. **Response timeline:**
   - Acknowledgement within 48 hours
   - Initial assessment within 7 days
   - Fix released within 30 days for confirmed vulnerabilities

4. **Credit:** You will be credited in the security advisory and release notes unless you prefer to remain anonymous.

## Security Measures

RustMail applies the following security practices:

- **CI auditing:** `cargo audit` runs on every push and pull request
- **Dependency pinning:** Workspace-level dependency management with locked versions
- **Input validation:** Bounded SMTP reads, FTS5 query sanitization, filename sanitization
- **Network security:** Default bind to `127.0.0.1`, configurable via `--bind`
- **CORS:** Origin-mirroring policy (not permissive)
- **Rate limiting:** Semaphore-based limits on SMTP sessions (100) and WebSocket connections (50)
- **Release safeguards:** Email release requires explicit `--release-host` flag with port allowlist
- **Security headers:** `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` on all responses
- **CSP:** Content Security Policy on HTML email preview iframes

## Scope

The following are in scope for security reports:

- SMTP server vulnerabilities (DoS, injection, buffer overflow)
- HTTP API vulnerabilities (XSS, CSRF, injection, SSRF)
- Authentication/authorization bypasses
- Information disclosure
- Dependency vulnerabilities

The following are out of scope:

- RustMail is a **development tool** — it is not designed to be exposed to the public internet
- Social engineering attacks
- Denial of service via legitimate high-volume email sending
