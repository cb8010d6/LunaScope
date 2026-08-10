const sensitiveAssignment = /((?:authorization|api[-_ ]?key|password|secret|access[_ -]?token|refresh[_ -]?token|cookie|credential(?:value|[_ -]?value)?)\s*[:=]\s*)(["'`]?)([^\s,;}\]"'`]+)/gi;
const bearerToken = /\bbearer\s+[^\s,;}\]"'`]+/gi;
const privateKey = /-----BEGIN\s+[A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END\s+[A-Z0-9 ]*PRIVATE KEY-----/gi;
const standaloneApiToken = /(^|[\s"'`])((?:sk|pk)-[A-Za-z0-9][A-Za-z0-9._~-]{15,})(?=$|[\s"'`,;])/gi;

export function redactSensitiveText(value, secrets = []) {
  let text = String(value ?? "");
  for (const secret of secrets) {
    if (typeof secret === "string" && secret.length > 0) {
      text = text.split(secret).join("[REDACTED]");
    }
  }
  return text
    .replace(privateKey, "[PRIVATE KEY REDACTED]")
    .replace(bearerToken, "Bearer [REDACTED]")
    .replace(sensitiveAssignment, "$1$2[REDACTED]")
    .replace(standaloneApiToken, "$1[REDACTED]");
}
