# Security Policy

## Reporting a vulnerability

Use GitHub's private vulnerability-reporting / Security Advisory flow for `LagrangeNSS/LunaScope` when it is available. Do not open a public issue containing exploit details, credentials, private workspace contents, or unredacted logs. If private reporting is unavailable, open a minimal public issue requesting a private contact channel without disclosing the vulnerability.

Include the affected LunaScope version, Windows version, reproduction boundary, impact, and redacted diagnostics. Do not include API keys, Authorization headers, cookies, Windows Credential Manager exports, signing certificates, or proprietary project files.

## Supported versions

| Version | Security fixes |
|---|---|
| Current `0.3.x` release/RC | Supported |
| Earlier preview versions | Upgrade before reporting unless the issue is migration-specific |

## Security-sensitive categories

- workspace/path escape or write-scope bypass;
- Tauri IPC/ACL or CSP bypass;
- credential, token, header, or diagnostic leakage;
- arbitrary process execution or child-process escape;
- worktree isolation or verified-integration bypass;
- unsafe extension/MCP import or install-hook execution;
- asset-protocol scope escape;
- side-effect replay after interruption or restart;
- Provider content treated as a higher-priority instruction.

The architecture and trust boundaries are documented in [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md). Third-party licensing is tracked separately in [docs/THIRD_PARTY_NOTICES.md](docs/THIRD_PARTY_NOTICES.md).

The repository currently has no final LunaScope distribution license. Repository visibility is not a license grant.
