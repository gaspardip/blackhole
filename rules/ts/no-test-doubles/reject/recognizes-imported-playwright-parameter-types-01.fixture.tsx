import type {Page as BrowserPage} from "@playwright/test"; async function attach(page: BrowserPage) { await page.route("**/*", route => route.fulfill({body: "fake"})); }
