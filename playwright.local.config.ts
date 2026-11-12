// Local only, never committed: the repository's config with the system
// Chromium, since Playwright's own browsers are not installed on this machine.
import base from "./playwright.config";

export default {
  ...base,
  retries: 1,
  workers: Number(process.env.E2E_WORKERS) || 3,
  use: { ...base.use, trace: "off" as const },
  projects: (base.projects ?? []).filter((p) => p.name === "chromium").map((p) => ({
    ...p,
    use: { ...p.use, launchOptions: { executablePath: "/usr/bin/chromium" } },
  })),
};
