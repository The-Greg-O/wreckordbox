#!/usr/bin/env node

/**
 * Upstream's `npm run deploy` script dispatched `Release` on `chrisle/rbxport`.
 * wreckordbox is a personal fork with no automated release or CDN pipeline;
 * keep this entry point as an explicit refusal so `pnpm deploy` can never
 * target upstream's repository.
 */
console.error(
  "deploy: wreckordbox does not publish releases; upstream's Release workflow has been removed."
);
process.exitCode = 1;
