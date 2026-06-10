#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  const arg = process.argv[index];
  if (!arg.startsWith('--')) continue;
  const key = arg.slice(2);
  const next = process.argv[index + 1];
  if (next == null || next.startsWith('--')) {
    args.set(key, 'true');
  } else {
    args.set(key, next);
    index += 1;
  }
}

const expectedPath = args.get('expected');
const actualPath = args.get('actual');
const outPath = args.get('out');
const failUnder = args.has('fail-under') ? Number(args.get('fail-under')) : null;
const maxSelectedChannelDelta = args.has('max-selected-channel-delta') ? Number(args.get('max-selected-channel-delta')) : null;
const maxAlphaDelta = args.has('max-alpha-delta') ? Number(args.get('max-alpha-delta')) : null;
const metricName = args.get('metric') ?? 'rgba';
const metricNames = new Set([
  'rgba',
  'rgb-all',
  'rgb-opaque',
  'rgb-visible',
  'rgb-nonblack',
  'rgb-interior1px',
  'rgb-visible-interior1px',
  'rgb-nonblack-interior1px',
  'rgb-shared-nonblack-interior1px',
  'rgb-shared-nonblack-interior2px',
  'rgb-shared-nonblack-interior3px',
  'rgb-shared-nonblack-flat32-interior1px',
]);

if (!expectedPath || !actualPath) {
  console.error('usage: node tools/render-parity/compare-psnr.mjs --expected expected.rgba.json --actual actual.rgba.json [--out report.json] [--metric rgba|rgb-all|rgb-opaque|rgb-visible|rgb-nonblack|rgb-interior1px|rgb-visible-interior1px|rgb-nonblack-interior1px|rgb-shared-nonblack-interior1px|rgb-shared-nonblack-interior2px|rgb-shared-nonblack-interior3px|rgb-shared-nonblack-flat32-interior1px] [--fail-under 40] [--max-selected-channel-delta 2] [--max-alpha-delta 1]');
  process.exit(2);
}
if (failUnder != null && (!Number.isFinite(failUnder) || failUnder < 0.0)) {
  console.error(`invalid --fail-under: ${args.get('fail-under')}`);
  process.exit(2);
}
if (maxSelectedChannelDelta != null && (!Number.isInteger(maxSelectedChannelDelta) || maxSelectedChannelDelta < 0 || maxSelectedChannelDelta > 255)) {
  console.error(`invalid --max-selected-channel-delta: ${args.get('max-selected-channel-delta')}`);
  process.exit(2);
}
if (maxAlphaDelta != null && (!Number.isInteger(maxAlphaDelta) || maxAlphaDelta < 0 || maxAlphaDelta > 255)) {
  console.error(`invalid --max-alpha-delta: ${args.get('max-alpha-delta')}`);
  process.exit(2);
}
if (!metricNames.has(metricName)) {
  console.error(`invalid --metric: ${metricName}; expected one of ${Array.from(metricNames).join(', ')}`);
  process.exit(2);
}

const expected = readRgbaJson(expectedPath);
const actual = readRgbaJson(actualPath);
if (expected.width !== actual.width || expected.height !== actual.height) {
  console.error(`image dimensions differ: expected ${expected.width}x${expected.height}, actual ${actual.width}x${actual.height}`);
  process.exit(3);
}
if (expected.rgba.length !== actual.rgba.length) {
  console.error(`image buffer lengths differ: expected ${expected.rgba.length}, actual ${actual.rgba.length}`);
  process.exit(3);
}

const fullImage = compareChannels(() => true, [0, 1, 2, 3]);
const allRgb = compareChannels(() => true, [0, 1, 2]);
const opaqueRgb = compareChannels((pixel) => expected.rgba[pixel + 3] === 255 && actual.rgba[pixel + 3] === 255, [0, 1, 2]);
const visibleRgb = compareChannels((pixel) => expected.rgba[pixel + 3] > 0 || actual.rgba[pixel + 3] > 0, [0, 1, 2]);
const nonblackRgb = compareChannels((pixel) => isNonblack(pixel), [0, 1, 2]);
const interiorRgb = compareChannels((pixel) => isInteriorOpaque(pixel), [0, 1, 2]);
const visibleInteriorRgb = compareChannels((pixel) => isInteriorVisible(pixel), [0, 1, 2]);
const nonblackInteriorRgb = compareChannels((pixel) => isInteriorNonblack(pixel), [0, 1, 2]);
const sharedNonblackInteriorRgb = compareChannels((pixel) => isInteriorSharedNonblack(pixel), [0, 1, 2]);
const sharedNonblackInterior2pxRgb = compareChannels((pixel) => isInteriorSharedNonblack(pixel, 2), [0, 1, 2]);
const sharedNonblackInterior3pxRgb = compareChannels((pixel) => isInteriorSharedNonblack(pixel, 3), [0, 1, 2]);
const sharedNonblackFlat32InteriorRgb = compareChannels((pixel) => isFlatSharedNonblackInterior(pixel, 1, 32), [0, 1, 2]);
const alpha = alphaStats();
const selectedMetric = selectMetric(metricName, {
  rgba: fullImage,
  'rgb-all': allRgb,
  'rgb-opaque': opaqueRgb,
  'rgb-visible': visibleRgb,
  'rgb-nonblack': nonblackRgb,
  'rgb-interior1px': interiorRgb,
  'rgb-visible-interior1px': visibleInteriorRgb,
  'rgb-nonblack-interior1px': nonblackInteriorRgb,
  'rgb-shared-nonblack-interior1px': sharedNonblackInteriorRgb,
  'rgb-shared-nonblack-interior2px': sharedNonblackInterior2pxRgb,
  'rgb-shared-nonblack-interior3px': sharedNonblackInterior3pxRgb,
  'rgb-shared-nonblack-flat32-interior1px': sharedNonblackFlat32InteriorRgb,
});
const mse = fullImage.mse;
const psnr = fullImage.psnr;
const report = {
  expected: path.resolve(expectedPath),
  actual: path.resolve(actualPath),
  width: expected.width,
  height: expected.height,
  channels: 4,
  mse,
  psnr: Number.isFinite(psnr) ? psnr : 'Infinity',
  maxChannelDelta: fullImage.maxChannelDelta,
  maxPixelDelta: fullImage.maxPixelDelta,
  alpha,
  rgbAll: metricReport(allRgb),
  rgbOpaque: metricReport(opaqueRgb),
  rgbVisible: metricReport(visibleRgb),
  rgbNonblack: metricReport(nonblackRgb),
  rgbInterior1px: metricReport(interiorRgb),
  rgbVisibleInterior1px: metricReport(visibleInteriorRgb),
  rgbNonblackInterior1px: metricReport(nonblackInteriorRgb),
  rgbSharedNonblackInterior1px: metricReport(sharedNonblackInteriorRgb),
  rgbSharedNonblackInterior2px: metricReport(sharedNonblackInterior2pxRgb),
  rgbSharedNonblackInterior3px: metricReport(sharedNonblackInterior3pxRgb),
  rgbSharedNonblackFlat32Interior1px: metricReport(sharedNonblackFlat32InteriorRgb),
  selectedMetric: {
    name: metricName,
    ...metricReport(selectedMetric),
  },
  pass: passStatus(selectedMetric, alpha),
  thresholds: {
    failUnder,
    maxSelectedChannelDelta,
    maxAlphaDelta,
  },
  failUnder,
};

const json = `${JSON.stringify(report, null, 2)}\n`;
if (outPath) {
  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, json);
} else {
  process.stdout.write(json);
}

if (!report.pass) {
  if (failUnder != null && selectedMetric.psnr < failUnder) {
    console.error(`PSNR ${selectedMetric.psnr.toFixed(4)} dB for ${metricName} is below threshold ${failUnder} dB`);
  }
  if (maxSelectedChannelDelta != null && selectedMetric.maxChannelDelta > maxSelectedChannelDelta) {
    console.error(`max selected channel delta ${selectedMetric.maxChannelDelta} for ${metricName} exceeds threshold ${maxSelectedChannelDelta}`);
  }
  if (maxAlphaDelta != null && alpha.maxDelta > maxAlphaDelta) {
    console.error(`max alpha delta ${alpha.maxDelta} exceeds threshold ${maxAlphaDelta}`);
  }
  process.exit(4);
}

function passStatus(selectedMetric, alpha) {
  return (failUnder == null || selectedMetric.psnr >= failUnder)
    && (maxSelectedChannelDelta == null || selectedMetric.maxChannelDelta <= maxSelectedChannelDelta)
    && (maxAlphaDelta == null || alpha.maxDelta <= maxAlphaDelta);
}

function readRgbaJson(file) {
  const parsed = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (!Number.isInteger(parsed.width) || parsed.width <= 0) {
    throw new Error(`${file}: width must be a positive integer`);
  }
  if (!Number.isInteger(parsed.height) || parsed.height <= 0) {
    throw new Error(`${file}: height must be a positive integer`);
  }
  if (!Array.isArray(parsed.rgba)) {
    throw new Error(`${file}: rgba must be an array`);
  }
  const expectedLength = parsed.width * parsed.height * 4;
  if (parsed.rgba.length !== expectedLength) {
    throw new Error(`${file}: rgba length ${parsed.rgba.length} does not match ${expectedLength}`);
  }
  const rgba = parsed.rgba.map((value, index) => {
    if (!Number.isInteger(value) || value < 0 || value > 255) {
      throw new Error(`${file}: rgba[${index}] must be an integer in 0..255`);
    }
    return value;
  });
  return { width: parsed.width, height: parsed.height, rgba };
}

function compareChannels(includePixel, channels) {
  let squaredError = 0.0;
  let absoluteError = 0.0;
  let sampleCount = 0;
  let pixelCount = 0;
  let maxChannelDelta = 0;
  let maxPixelDelta = 0.0;
  for (let offset = 0; offset < expected.rgba.length; offset += 4) {
    if (!includePixel(offset)) continue;
    let pixelSquared = 0.0;
    for (const channel of channels) {
      const delta = actual.rgba[offset + channel] - expected.rgba[offset + channel];
      const absolute = Math.abs(delta);
      maxChannelDelta = Math.max(maxChannelDelta, absolute);
      squaredError += delta * delta;
      absoluteError += absolute;
      pixelSquared += delta * delta;
      sampleCount += 1;
    }
    pixelCount += 1;
    maxPixelDelta = Math.max(maxPixelDelta, Math.sqrt(pixelSquared));
  }
  if (sampleCount === 0) {
    return {
      pixelCount,
      channelCount: 0,
      mse: null,
      mae: null,
      psnr: null,
      maxChannelDelta,
      maxPixelDelta,
    };
  }
  const mse = squaredError / sampleCount;
  const mae = absoluteError / sampleCount;
  const psnr = mse === 0.0 ? Number.POSITIVE_INFINITY : 10.0 * Math.log10((255.0 * 255.0) / mse);
  return {
    pixelCount,
    channelCount: sampleCount,
    mse,
    mae,
    psnr,
    maxChannelDelta,
    maxPixelDelta,
  };
}

function metricReport(metric) {
  return {
    pixels: metric.pixelCount,
    channels: metric.channelCount,
    mse: metric.mse,
    mae: metric.mae,
    psnr: metric.psnr == null ? null : Number.isFinite(metric.psnr) ? metric.psnr : 'Infinity',
    maxChannelDelta: metric.maxChannelDelta,
    maxPixelDelta: metric.maxPixelDelta,
  };
}

function selectMetric(name, metrics) {
  const metric = metrics[name];
  if (metric.channelCount === 0) {
    throw new Error(`selected metric ${name} has no comparable pixels`);
  }
  return metric;
}

function alphaStats() {
  const expectedCounts = { transparent: 0, opaque: 0, partial: 0 };
  const actualCounts = { transparent: 0, opaque: 0, partial: 0 };
  let mismatches = 0;
  let maxDelta = 0;
  let mismatchesBeyondOne = 0;
  for (let offset = 0; offset < expected.rgba.length; offset += 4) {
    countAlpha(expectedCounts, expected.rgba[offset + 3]);
    countAlpha(actualCounts, actual.rgba[offset + 3]);
    const delta = Math.abs(expected.rgba[offset + 3] - actual.rgba[offset + 3]);
    maxDelta = Math.max(maxDelta, delta);
    if (delta !== 0) {
      mismatches += 1;
    }
    if (delta > 1) {
      mismatchesBeyondOne += 1;
    }
  }
  return {
    expected: expectedCounts,
    actual: actualCounts,
    mismatches,
    maxDelta,
    mismatchesBeyondOne,
  };
}

function countAlpha(counts, alpha) {
  if (alpha === 0) {
    counts.transparent += 1;
  } else if (alpha === 255) {
    counts.opaque += 1;
  } else {
    counts.partial += 1;
  }
}

function isInteriorOpaque(pixel) {
  const pixelIndex = pixel / 4;
  const x = pixelIndex % expected.width;
  const y = Math.floor(pixelIndex / expected.width);
  if (x === 0 || y === 0 || x === expected.width - 1 || y === expected.height - 1) {
    return false;
  }
  for (let dy = -1; dy <= 1; dy += 1) {
    for (let dx = -1; dx <= 1; dx += 1) {
      const neighbor = ((y + dy) * expected.width + (x + dx)) * 4 + 3;
      if (expected.rgba[neighbor] !== 255 || actual.rgba[neighbor] !== 255) {
        return false;
      }
    }
  }
  return true;
}

function isInteriorVisible(pixel) {
  const pixelIndex = pixel / 4;
  const x = pixelIndex % expected.width;
  const y = Math.floor(pixelIndex / expected.width);
  if (x === 0 || y === 0 || x === expected.width - 1 || y === expected.height - 1) {
    return false;
  }
  for (let dy = -1; dy <= 1; dy += 1) {
    for (let dx = -1; dx <= 1; dx += 1) {
      const neighbor = ((y + dy) * expected.width + (x + dx)) * 4 + 3;
      if (expected.rgba[neighbor] === 0 || actual.rgba[neighbor] === 0) {
        return false;
      }
    }
  }
  return true;
}

function isNonblack(pixel) {
  return pixelRgbNonzero(expected.rgba, pixel) || pixelRgbNonzero(actual.rgba, pixel);
}

function isInteriorNonblack(pixel) {
  const pixelIndex = pixel / 4;
  const x = pixelIndex % expected.width;
  const y = Math.floor(pixelIndex / expected.width);
  if (x === 0 || y === 0 || x === expected.width - 1 || y === expected.height - 1) {
    return false;
  }
  for (let dy = -1; dy <= 1; dy += 1) {
    for (let dx = -1; dx <= 1; dx += 1) {
      const neighbor = ((y + dy) * expected.width + (x + dx)) * 4;
      if (!isNonblack(neighbor)) {
        return false;
      }
    }
  }
  return true;
}

function isInteriorSharedNonblack(pixel, radius = 1) {
  const pixelIndex = pixel / 4;
  const x = pixelIndex % expected.width;
  const y = Math.floor(pixelIndex / expected.width);
  if (x < radius || y < radius || x + radius >= expected.width || y + radius >= expected.height) {
    return false;
  }
  for (let dy = -radius; dy <= radius; dy += 1) {
    for (let dx = -radius; dx <= radius; dx += 1) {
      const neighbor = ((y + dy) * expected.width + (x + dx)) * 4;
      if (!isSharedNonblack(neighbor)) {
        return false;
      }
    }
  }
  return true;
}

function isSharedNonblack(pixel) {
  return pixelRgbNonzero(expected.rgba, pixel) && pixelRgbNonzero(actual.rgba, pixel);
}

function isFlatSharedNonblackInterior(pixel, radius, maxChannelDelta) {
  const pixelIndex = pixel / 4;
  const x = pixelIndex % expected.width;
  const y = Math.floor(pixelIndex / expected.width);
  if (x < radius || y < radius || x + radius >= expected.width || y + radius >= expected.height) {
    return false;
  }
  for (let dy = -radius; dy <= radius; dy += 1) {
    for (let dx = -radius; dx <= radius; dx += 1) {
      const neighbor = ((y + dy) * expected.width + (x + dx)) * 4;
      if (!isSharedNonblack(neighbor)
        || rgbMaxDelta(expected.rgba, pixel, neighbor) > maxChannelDelta
        || rgbMaxDelta(actual.rgba, pixel, neighbor) > maxChannelDelta) {
        return false;
      }
    }
  }
  return true;
}

function rgbMaxDelta(rgba, left, right) {
  return Math.max(
    Math.abs(rgba[left] - rgba[right]),
    Math.abs(rgba[left + 1] - rgba[right + 1]),
    Math.abs(rgba[left + 2] - rgba[right + 2]),
  );
}

function pixelRgbNonzero(rgba, pixel) {
  return rgba[pixel] !== 0 || rgba[pixel + 1] !== 0 || rgba[pixel + 2] !== 0;
}
