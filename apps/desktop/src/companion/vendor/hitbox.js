export function calculateInteractiveBounds({
  width,
  height,
  left,
  top,
  modelX,
  modelY,
  userScale = 1,
  hitboxPadding = 8
}) {
  const safeWidth = Math.max(1, Number(width) || 0);
  const safeHeight = Math.max(1, Number(height) || 0);
  const hitWidth = Math.max(44, safeWidth);
  const hitHeight = Math.max(72, safeHeight);
  const padding = Math.max(4, Math.min(24, Number(hitboxPadding || 0) * Math.max(0.7, Number(userScale) || 1)));
  const exactLeft = Number(left);
  const exactTop = Number(top);
  const centerX = Number.isFinite(exactLeft) ? exactLeft + safeWidth / 2 : Number(modelX || 0);
  const centerY = Number.isFinite(exactTop) ? exactTop + safeHeight / 2 : Number(modelY || 0) - safeHeight / 2;
  return {
    left: centerX - hitWidth / 2 - padding,
    right: centerX + hitWidth / 2 + padding,
    top: centerY - hitHeight / 2 - padding,
    bottom: centerY + hitHeight / 2 + padding
  };
}

export function transformLocalBounds(bounds, transform = {}) {
  if (!bounds) return null;
  const width = Number(bounds.width);
  const height = Number(bounds.height);
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) return null;
  const scaleX = Number.isFinite(Number(transform.scaleX)) ? Number(transform.scaleX) : 1;
  const scaleY = Number.isFinite(Number(transform.scaleY)) ? Number(transform.scaleY) : 1;
  const x = Number(bounds.x || 0) + Number(transform.childX || 0);
  const y = Number(bounds.y || 0) + Number(transform.childY || 0);
  const firstX = Number(transform.x || 0) + x * scaleX;
  const secondX = Number(transform.x || 0) + (x + width) * scaleX;
  const firstY = Number(transform.y || 0) + y * scaleY;
  const secondY = Number(transform.y || 0) + (y + height) * scaleY;
  return {
    left: Math.min(firstX, secondX),
    right: Math.max(firstX, secondX),
    top: Math.min(firstY, secondY),
    bottom: Math.max(firstY, secondY),
    width: Math.abs(secondX - firstX),
    height: Math.abs(secondY - firstY)
  };
}

export function expandBounds(bounds, padding = 18) {
  if (!bounds) return null;
  const amount = Math.max(0, Number(padding) || 0);
  return {
    left: bounds.left - amount,
    right: bounds.right + amount,
    top: bounds.top - amount,
    bottom: bounds.bottom + amount
  };
}

export function normalizePointerRegions(input, maxRegions = 16) {
  const limit = Math.max(1, Math.min(16, Math.floor(Number(maxRegions) || 16)));
  const candidates = Array.isArray(input) ? input : input ? [input] : [];
  return candidates
    .map((bounds) => ({
      left: Number(bounds?.left),
      right: Number(bounds?.right),
      top: Number(bounds?.top),
      bottom: Number(bounds?.bottom)
    }))
    .filter((bounds) => Object.values(bounds).every(Number.isFinite)
      && bounds.right > bounds.left
      && bounds.bottom > bounds.top)
    .slice(0, limit);
}

function regionArea(region) {
  return Math.max(0, region.right - region.left) * Math.max(0, region.bottom - region.top);
}

function mergeRegionPair(left, right) {
  return {
    left: Math.min(left.left, right.left),
    right: Math.max(left.right, right.right),
    top: Math.min(left.top, right.top),
    bottom: Math.max(left.bottom, right.bottom)
  };
}

function regionsTouch(left, right, gap = 3) {
  const horizontalOverlap = Math.min(left.right, right.right) - Math.max(left.left, right.left);
  const verticalOverlap = Math.min(left.bottom, right.bottom) - Math.max(left.top, right.top);
  if (horizontalOverlap >= 0 && verticalOverlap >= 0) return true;
  const leftHeight = left.bottom - left.top;
  const rightHeight = right.bottom - right.top;
  const leftWidth = left.right - left.left;
  const rightWidth = right.right - right.left;
  const horizontalGap = Math.max(0, Math.max(left.left, right.left) - Math.min(left.right, right.right));
  const verticalGap = Math.max(0, Math.max(left.top, right.top) - Math.min(left.bottom, right.bottom));
  return (horizontalGap <= gap && verticalOverlap >= Math.min(leftHeight, rightHeight) * 0.35)
    || (verticalGap <= gap && horizontalOverlap >= Math.min(leftWidth, rightWidth) * 0.35);
}

export function compactPointerRegions(input, { maxRegions = 16, padding = 3, mergeGap = 3 } = {}) {
  const amount = Math.max(0, Math.min(12, Number(padding) || 0));
  const candidates = normalizePointerRegions(input, 64)
    .map((region) => ({
      left: region.left - amount,
      right: region.right + amount,
      top: region.top - amount,
      bottom: region.bottom + amount
    }))
    .filter((region) => regionArea(region) >= 9)
    .sort((left, right) => regionArea(right) - regionArea(left));

  const merged = [];
  for (const candidate of candidates) {
    const index = merged.findIndex((region) => regionsTouch(region, candidate, mergeGap));
    if (index < 0) merged.push(candidate);
    else merged[index] = mergeRegionPair(merged[index], candidate);
  }

  let changed = true;
  while (changed) {
    changed = false;
    for (let left = 0; left < merged.length && !changed; left += 1) {
      for (let right = left + 1; right < merged.length; right += 1) {
        if (!regionsTouch(merged[left], merged[right], mergeGap)) continue;
        merged[left] = mergeRegionPair(merged[left], merged[right]);
        merged.splice(right, 1);
        changed = true;
        break;
      }
    }
  }

  return merged
    .sort((left, right) => regionArea(right) - regionArea(left))
    .slice(0, Math.max(1, Math.min(16, Number(maxRegions) || 16)));
}

export function unionPointerRegions(input) {
  const regions = normalizePointerRegions(input);
  if (!regions.length) return null;
  return regions.reduce((union, region) => mergeRegionPair(union, region));
}
