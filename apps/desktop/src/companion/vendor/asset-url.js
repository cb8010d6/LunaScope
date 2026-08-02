export function encodeSpineAssetUrl(url) {
  return String(url || "").replace(/#/g, "%23");
}

export function spineAssetDirectoryUrl(url) {
  const encoded = encodeSpineAssetUrl(url)
    .replace(/%5c/gi, "/")
    .replace(/\\/g, "/");
  const lastSlash = encoded.lastIndexOf("/");
  return lastSlash >= 0 ? encoded.slice(0, lastSlash + 1) : "";
}

export function joinSpineAssetUrl(baseUrl, relativePath) {
  const base = spineAssetDirectoryUrl(baseUrl) || String(baseUrl || "");
  const relative = String(relativePath || "").replace(/\\/g, "/").replace(/#/g, "%23");
  try {
    return new URL(relative, base).href;
  } catch {
    return `${base}${relative}`;
  }
}

export function spineAssetUrl(config = {}) {
  const explicit = config.spine?.assetUrl;
  if (explicit) return encodeSpineAssetUrl(explicit);

  const origin = String(config.server?.origin || "http://127.0.0.1:17388").replace(/\/$/, "");
  const skel = String(config.spine?.skel || "");
  return `${origin}/assets/spine/${encodeURIComponent(skel)}`;
}
