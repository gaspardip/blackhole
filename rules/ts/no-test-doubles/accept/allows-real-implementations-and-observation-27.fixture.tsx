import { runInDurableObject } from "cloudflare:test"; runInDurableObject(stub, (instance) => { function configure(instance) { instance.fetch = () => null; } });
