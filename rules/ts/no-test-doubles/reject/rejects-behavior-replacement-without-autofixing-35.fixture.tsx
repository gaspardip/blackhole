import { runInDurableObject } from "cloudflare:test"; runInDurableObject(stub, (instance, state) => { const { storage } = state; Object.assign(storage, { put: async () => {} }); });
