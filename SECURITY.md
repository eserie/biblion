# Security Policy

## Reporting a Vulnerability

Please email **security@biblion.dev** with:

1. Description of the vulnerability
2. Steps to reproduce
3. Potential impact
4. Suggested fix (if any)

Do **not** disclose publicly until a fix is released or 90 days have passed.

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Security Updates

Security patches are released as PATCH version bumps and published to crates.io immediately.

## Scope

This project reads Zotero's SQLite database in read-only mode. Write operations go through the Zotero Web API with an API key. The API key is provided via environment variable and never stored on disk by this tool.

The PDF resolver makes HTTP requests to 9 academic APIs. It does not execute downloaded content.
