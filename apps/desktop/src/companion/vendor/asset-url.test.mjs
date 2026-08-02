import test from "node:test";
import assert from "node:assert/strict";

import { joinSpineAssetUrl, spineAssetDirectoryUrl } from "./asset-url.js";

test("keeps the Tauri asset directory for atlas pages containing a hash", () => {
  const atlasUrl = "http://asset.localhost/D%3A/LunaScopeData/companion/models/amiya/dyn_illust_sale%2313.atlas";

  assert.equal(
    spineAssetDirectoryUrl(atlasUrl),
    "http://asset.localhost/D%3A/LunaScopeData/companion/models/amiya/",
  );
});

test("joins a hash-containing atlas page without losing the model directory", () => {
  const atlasUrl = "http://asset.localhost/D%3A/models/amiya/dyn_illust_sale%2313.atlas";

  assert.equal(
    joinSpineAssetUrl(atlasUrl, "dyn_illust_sale#13.png"),
    "http://asset.localhost/D%3A/models/amiya/dyn_illust_sale%2313.png",
  );
});

test("falls back to the skeleton directory when no explicit atlas is configured", () => {
  assert.equal(
    spineAssetDirectoryUrl("http://asset.localhost/D%3A/models/amiya/model.skel"),
    "http://asset.localhost/D%3A/models/amiya/",
  );
});

test("normalizes Tauri Windows URLs whose separators are encoded backslashes", () => {
  const atlasUrl = "http://asset.localhost/D%3A%5CLunaScopeData%5Ccompanion%5Cmodels%5Camiya2%5Cdyn_illust_char_1001_amiya2_sale%2316.atlas";

  assert.equal(
    joinSpineAssetUrl(atlasUrl, "dyn_illust_char_1001_amiya2_sale#16.png"),
    "http://asset.localhost/D%3A/LunaScopeData/companion/models/amiya2/dyn_illust_char_1001_amiya2_sale%2316.png",
  );
});
