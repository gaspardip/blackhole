import { runInDurableObject } from "cloudflare:test"; function send() {} runInDurableObject(stub, (instance) => { instance.send = send; });
