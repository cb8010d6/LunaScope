export function encodeSpineAssetUrl(url: string): string;
export function spineAssetDirectoryUrl(url: string): string;
export function joinSpineAssetUrl(baseUrl: string, relativePath: string): string;

export function spineAssetUrl(config?: {
  server?: { origin?: string };
  spine?: { assetUrl?: string; skel?: string };
}): string;
