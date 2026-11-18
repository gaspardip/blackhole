import { runInDurableObject } from "cloudflare:test"; runInDurableObject(stub, (instance) => { instance.env.TOKEN = "token"; });
