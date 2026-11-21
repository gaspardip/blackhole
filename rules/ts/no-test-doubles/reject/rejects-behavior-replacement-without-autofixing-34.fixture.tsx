import { runInDurableObject } from "cloudflare:test"; runInDurableObject(stub, (instance) => { const target = instance; Reflect.set(target, "fetch", () => null); });
