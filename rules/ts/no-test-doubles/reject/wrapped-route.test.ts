import type { Page } from "playwright"; function attach(page: Page) { page.route(url, (route => route.fulfill({body:"fake"}))); }
