import type {Route} from "playwright"; async function handler(route: Route) { await route.fulfill({status: 503}); }
