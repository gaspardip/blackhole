import { vi } from "vitest"; vi.stubGlobal("fetch", async () => new Response());
