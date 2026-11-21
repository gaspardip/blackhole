import { runInDurableObject } from "cloudflare:test"; runInDurableObject(stub, (instance) => { vi.spyOn(instance, "fetch"); });
