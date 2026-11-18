import { runInDurableObject } from "cloudflare:test"; function configure(runInDurableObject) { runInDurableObject(stub, (instance) => { instance.fetch = () => null; }); }
