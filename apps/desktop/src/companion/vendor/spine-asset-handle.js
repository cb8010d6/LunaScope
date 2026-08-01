import * as PIXI from "pixi.js";

const DEFAULT_RESOURCE_NAME = "spine-companion-asset";

function cancellationError(identity) {
  const error = new Error(`Spine asset load was cancelled: ${identity}`);
  error.name = "AbortError";
  error.code = "SPINE_ASSET_CANCELLED";
  return error;
}

function cacheObjects(pixi, name) {
  return [...new Set([
    pixi?.utils?.[name],
    pixi?.[name]
  ].filter(Boolean))];
}

function cacheValues(pixi, name) {
  const values = new Set();
  for (const cache of cacheObjects(pixi, name)) {
    for (const value of Object.values(cache)) values.add(value);
  }
  return values;
}

function fallbackResources(pixi) {
  const textures = new Set();
  const baseTextures = new Set();
  for (const name of ["EMPTY", "WHITE"]) {
    try {
      const texture = pixi?.Texture?.[name];
      if (!texture) continue;
      textures.add(texture);
      if (texture.baseTexture) baseTextures.add(texture.baseTexture);
    } catch {
      // Some headless Pixi builds lazily create fallback textures through a DOM canvas.
    }
  }
  return { textures, baseTextures };
}

function evictFromCaches(pixi, value, cacheName, ownerType) {
  const caches = cacheObjects(pixi, cacheName);
  const removeFromCache = pixi?.[ownerType]?.removeFromCache;
  for (const cache of caches) {
    for (const key of Object.keys(cache)) {
      if (cache[key] !== value) continue;
      try {
        removeFromCache?.call(pixi[ownerType], key);
      } catch {
        // Direct deletion below still avoids retaining a disposed resource.
      }
      if (cache[key] === value) delete cache[key];
    }
  }
}

function destroyTexture(pixi, texture) {
  evictFromCaches(pixi, texture, "TextureCache", "Texture");
  try { texture.destroy?.(false); } catch { /* cleanup is best-effort */ }
}

function destroyBaseTexture(pixi, baseTexture) {
  evictFromCaches(pixi, baseTexture, "BaseTextureCache", "BaseTexture");
  try { baseTexture.destroy?.(); } catch { /* cleanup is best-effort */ }
}

function collectResourceTextures(resources) {
  const textures = new Set();
  for (const resource of Object.values(resources || {})) {
    if (resource?.texture) textures.add(resource.texture);
    for (const texture of Object.values(resource?.textures || {})) {
      if (texture) textures.add(texture);
    }
  }
  return textures;
}

function collectAtlases(resource, resources) {
  const atlases = new Set();
  if (resource?.spineAtlas) atlases.add(resource.spineAtlas);
  for (const candidate of Object.values(resources || {})) {
    if (candidate?.spineAtlas) atlases.add(candidate.spineAtlas);
  }
  return atlases;
}

function collectSpineData(resource, resources) {
  const spineData = new Set();
  if (resource?.spineData) spineData.add(resource.spineData);
  for (const candidate of Object.values(resources || {})) {
    if (candidate?.spineData) spineData.add(candidate.spineData);
  }
  return spineData;
}

function discoverOwnership(entry, pixi, textureRefs, baseTextureRefs) {
  const resources = entry.resources || entry.loader?.resources || {};
  const resourceTextures = collectResourceTextures(resources);
  const resourceBaseTextures = new Set(
    [...resourceTextures].map((texture) => texture?.baseTexture).filter(Boolean)
  );
  const explicitTextures = new Set(entry.loadResult?.ownedTextures || []);
  const explicitBaseTextures = new Set(entry.loadResult?.ownedBaseTextures || []);
  const atlases = collectAtlases(entry.resource, resources);
  const spineData = collectSpineData(entry.resource, resources);
  const { textures: fallbackTextures, baseTextures: fallbackBaseTextures } = fallbackResources(pixi);

  const ownedBaseTextures = new Set();
  const pageBaseTextures = new Set();
  for (const atlas of atlases) {
    for (const page of atlas?.pages || []) {
      if (page?.baseTexture) pageBaseTextures.add(page.baseTexture);
    }
  }
  for (const baseTexture of [...pageBaseTextures, ...explicitBaseTextures]) {
    const cameFromLoader = resourceBaseTextures.has(baseTexture) || explicitBaseTextures.has(baseTexture);
    if (cameFromLoader
      && (!entry.preexistingBaseTextures.has(baseTexture) || baseTextureRefs.has(baseTexture))
      && !fallbackBaseTextures.has(baseTexture)) {
      ownedBaseTextures.add(baseTexture);
    }
  }

  const ownedTextures = new Set();
  for (const texture of [...resourceTextures, ...explicitTextures]) {
    if ((!entry.preexistingTextures.has(texture) || textureRefs.has(texture))
      && !fallbackTextures.has(texture)) {
      ownedTextures.add(texture);
    }
  }
  for (const atlas of atlases) {
    for (const region of atlas?.regions || []) {
      const texture = region?.texture;
      if (texture
        && ownedBaseTextures.has(texture.baseTexture)
        && !entry.preexistingTextures.has(texture)
        && !fallbackTextures.has(texture)) {
        ownedTextures.add(texture);
      }
    }
  }

  return { atlases, spineData, ownedTextures, ownedBaseTextures };
}

function defaultLoadAsset(identity, context) {
  const Loader = context.pixi?.Loader;
  if (typeof Loader !== "function") {
    throw new Error("PIXI.Loader is unavailable; provide a loadAsset adapter.");
  }

  const loader = new Loader();
  context.setLoader(loader);
  return new Promise((resolve, reject) => {
    let settled = false;
    const fail = (error) => {
      if (settled) return;
      settled = true;
      reject(error instanceof Error ? error : new Error(String(error || "Unable to load Spine asset.")));
    };

    loader.onError?.add?.(fail);
    loader.add(context.resourceName, context.url);
    loader.load((_loader, resources) => {
      if (settled) return;
      const resource = resources?.[context.resourceName];
      if (!resource?.spineData) {
        fail(new Error("Spine data was not found in the loaded asset."));
        return;
      }
      settled = true;
      resolve({ loader, resources, resource });
    });
  });
}

export function spineAssetIdentity(resourceIdentity) {
  if (resourceIdentity instanceof URL) return resourceIdentity.href;
  if (typeof resourceIdentity === "string") {
    const identity = resourceIdentity.trim();
    if (identity) return identity;
  }

  const source = resourceIdentity || {};
  for (const candidate of [
    source.resourceIdentity,
    source.spinePreviewUrl,
    source.assetUrl,
    source.url,
    source.spine?.resourceIdentity,
    source.spine?.assetUrl,
    source.spine?.url
  ]) {
    if (typeof candidate === "string" && candidate.trim()) return candidate.trim();
  }

  const spine = source.spine || source;
  const assetDir = typeof spine.assetDir === "string" ? spine.assetDir.trim().replace(/\\/g, "/").replace(/\/$/, "") : "";
  const skel = typeof spine.skel === "string" ? spine.skel.trim().replace(/^\//, "") : "";
  if (assetDir && skel) return `${assetDir}/${skel}`;
  throw new TypeError("A Spine model resource identity or URL is required.");
}

export class SpineAssetHandle {
  constructor(registry, entry) {
    this.registry = registry;
    this.entry = entry;
    this.released = false;
    this.waiters = new Set();
  }

  get identity() {
    return this.entry.key;
  }

  get resource() {
    return this.released ? null : this.entry.resource;
  }

  load(options = {}) {
    if (this.released) return Promise.reject(cancellationError(this.identity));
    const signal = options.signal;
    if (signal?.aborted) return Promise.reject(cancellationError(this.identity));
    const sharedLoad = this.registry.loadEntry(this.entry);

    return new Promise((resolve, reject) => {
      let settled = false;
      const finish = (callback, value) => {
        if (settled) return;
        settled = true;
        this.waiters.delete(cancel);
        signal?.removeEventListener?.("abort", cancel);
        callback(value);
      };
      const cancel = () => finish(reject, cancellationError(this.identity));
      this.waiters.add(cancel);
      signal?.addEventListener?.("abort", cancel, { once: true });
      sharedLoad.then(
        (resource) => finish(resolve, resource),
        (error) => finish(reject, error)
      );
    });
  }

  release() {
    if (this.released) return;
    this.released = true;
    for (const cancel of [...this.waiters]) cancel();
    this.registry.releaseEntry(this.entry);
  }
}

export class SpineAssetRegistry {
  constructor(options = {}) {
    this.pixi = options.pixi || PIXI;
    this.loadAsset = options.loadAsset || defaultLoadAsset;
    this.entries = new Map();
    this.textureRefs = new WeakMap();
    this.baseTextureRefs = new WeakMap();
    this.metrics = { loadsStarted: 0, loadsSucceeded: 0, loadsFailed: 0, cleanups: 0 };
  }

  acquire(resourceIdentity, options = {}) {
    const identity = spineAssetIdentity(resourceIdentity);
    const key = options.key ? String(options.key) : identity;
    let entry = this.entries.get(key);
    if (!entry) {
      entry = {
        key,
        identity: resourceIdentity,
        url: options.url || identity,
        resourceName: options.resourceName || DEFAULT_RESOURCE_NAME,
        refs: 0,
        status: "idle",
        promise: null,
        loader: null,
        resources: null,
        resource: null,
        loadResult: null,
        ownership: null,
        preexistingTextures: new Set(),
        preexistingBaseTextures: new Set(),
        disposed: false
      };
      this.entries.set(key, entry);
    }
    entry.refs += 1;
    return new SpineAssetHandle(this, entry);
  }

  loadEntry(entry) {
    if (entry.disposed) return Promise.reject(cancellationError(entry.key));
    if (entry.promise) return entry.promise;

    entry.status = "loading";
    entry.preexistingTextures = cacheValues(this.pixi, "TextureCache");
    entry.preexistingBaseTextures = cacheValues(this.pixi, "BaseTextureCache");
    this.metrics.loadsStarted += 1;
    entry.promise = Promise.resolve().then(async () => {
      try {
        const result = await this.loadAsset(entry.identity, {
          key: entry.key,
          url: entry.url,
          resourceName: entry.resourceName,
          pixi: this.pixi,
          setLoader: (loader) => { entry.loader = loader; }
        });
        entry.loadResult = result?.resource ? result : { resource: result };
        entry.loader = entry.loadResult.loader || entry.loader;
        entry.resources = entry.loadResult.resources || entry.loader?.resources || null;
        entry.resource = entry.loadResult.resource;
        if (!entry.resource?.spineData) {
          throw new Error("Spine data was not found in the loaded asset.");
        }
        entry.ownership = discoverOwnership(entry, this.pixi, this.textureRefs, this.baseTextureRefs);
        this.retainOwnership(entry.ownership);
        entry.status = "ready";
        this.metrics.loadsSucceeded += 1;
        const resource = entry.resource;
        if (entry.refs === 0 || entry.disposed) this.cleanupEntry(entry);
        return resource;
      } catch (error) {
        entry.status = "failed";
        this.metrics.loadsFailed += 1;
        if (!entry.ownership) {
          entry.ownership = discoverOwnership(entry, this.pixi, this.textureRefs, this.baseTextureRefs);
          this.retainOwnership(entry.ownership);
        }
        this.cleanupEntry(entry);
        throw error;
      }
    });
    return entry.promise;
  }

  releaseEntry(entry) {
    entry.refs = Math.max(0, entry.refs - 1);
    if (entry.refs === 0 && (entry.status === "idle" || entry.status === "ready" || entry.status === "failed")) {
      this.cleanupEntry(entry);
    }
  }

  retainOwnership(ownership) {
    for (const texture of ownership.ownedTextures) {
      this.textureRefs.set(texture, (this.textureRefs.get(texture) || 0) + 1);
    }
    for (const baseTexture of ownership.ownedBaseTextures) {
      this.baseTextureRefs.set(baseTexture, (this.baseTextureRefs.get(baseTexture) || 0) + 1);
    }
  }

  cleanupEntry(entry) {
    if (entry.disposed) return;
    entry.disposed = true;
    this.metrics.cleanups += 1;
    if (this.entries.get(entry.key) === entry) this.entries.delete(entry.key);

    const ownership = entry.ownership
      || discoverOwnership(entry, this.pixi, this.textureRefs, this.baseTextureRefs);
    for (const texture of ownership.ownedTextures) {
      const refs = Math.max(0, (this.textureRefs.get(texture) || 1) - 1);
      if (refs > 0) this.textureRefs.set(texture, refs);
      else {
        this.textureRefs.delete(texture);
        destroyTexture(this.pixi, texture);
      }
    }
    for (const baseTexture of ownership.ownedBaseTextures) {
      const refs = Math.max(0, (this.baseTextureRefs.get(baseTexture) || 1) - 1);
      if (refs > 0) this.baseTextureRefs.set(baseTexture, refs);
      else {
        this.baseTextureRefs.delete(baseTexture);
        destroyBaseTexture(this.pixi, baseTexture);
      }
    }

    for (const spineData of ownership.spineData) {
      try { spineData?.dispose?.(); } catch { /* cleanup is best-effort */ }
    }
    for (const atlas of ownership.atlases) {
      if (Array.isArray(atlas?.regions)) atlas.regions.length = 0;
      if (Array.isArray(atlas?.pages)) atlas.pages.length = 0;
    }
    try { entry.loader?.destroy?.(); } catch { /* cleanup is best-effort */ }

    entry.loader = null;
    entry.resources = null;
    entry.resource = null;
    entry.loadResult = null;
    entry.ownership = null;
  }

  stats() {
    const entries = [...this.entries.values()];
    return {
      entries: entries.length,
      refs: entries.reduce((total, entry) => total + entry.refs, 0),
      idle: entries.filter((entry) => entry.status === "idle").length,
      loading: entries.filter((entry) => entry.status === "loading").length,
      ready: entries.filter((entry) => entry.status === "ready").length,
      ...this.metrics
    };
  }

  resetForTests() {
    for (const entry of [...this.entries.values()]) {
      entry.refs = 0;
      if (entry.status !== "loading") this.cleanupEntry(entry);
    }
    this.entries.clear();
    this.metrics = { loadsStarted: 0, loadsSucceeded: 0, loadsFailed: 0, cleanups: 0 };
  }
}

export const spineAssetRegistry = new SpineAssetRegistry();

export function acquireSpineAsset(resourceIdentity, options) {
  return spineAssetRegistry.acquire(resourceIdentity, options);
}

export function getSpineAssetRegistryStatsForTests() {
  return spineAssetRegistry.stats();
}

export function resetSpineAssetRegistryForTests() {
  spineAssetRegistry.resetForTests();
}
