export function encodeSpineAssetUrl(url) {
  return String(url || "").replace(/#/g, "%23");
}

export function spineAssetUrl(config = {}) {
  const explicit = config.spine?.assetUrl;
  if (explicit) return encodeSpineAssetUrl(explicit);

  const origin = String(config.server?.origin || "http://127.0.0.1:17388").replace(/\/$/, "");
  const skel = String(config.spine?.skel || "");
  return `${origin}/assets/spine/${encodeURIComponent(skel)}`;
}
