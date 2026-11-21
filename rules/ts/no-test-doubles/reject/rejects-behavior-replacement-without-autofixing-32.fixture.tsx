import { runInDurableObject } from "cloudflare:test"; runInDurableObject(stub, async (instance) => { instance.containerFetch = async () => new Response(); });
