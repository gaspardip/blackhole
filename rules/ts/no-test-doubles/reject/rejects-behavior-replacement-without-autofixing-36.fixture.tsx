import { runInDurableObject } from "cloudflare:test"; const run = runInDurableObject; run(stub, function (instance) { Object.defineProperty(instance, "fetch", { value: () => null }); });
