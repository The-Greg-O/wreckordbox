// Local only, never committed: the system Chromium, since Playwright's own browsers are not installed here.
import { devices } from "@playwright/test";
import base from "./playwright.config";
export default {
  ...base,
  projects: [{
    name: "chromium",
    use: { ...devices["Desktop Chrome"], viewport: { width: 1800, height: 1130 }, launchOptions: { executablePath: "/usr/bin/chromium" } },
  }],
};
