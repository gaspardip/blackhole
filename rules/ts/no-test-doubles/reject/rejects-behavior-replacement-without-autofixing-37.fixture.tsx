import { runInDurableObject } from "cloudflare:test"; runInDurableObject.call(null, stub, (instance) => { instance.fetch = () => null; });
