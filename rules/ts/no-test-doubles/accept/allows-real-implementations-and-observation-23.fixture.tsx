import { runInDurableObject } from "cloudflare:test"; runInDurableObject(stub, async (instance, state) => { await state.storage.put("key", "value"); });
