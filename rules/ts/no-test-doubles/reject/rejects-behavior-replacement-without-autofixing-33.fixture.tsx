import { runInDurableObject as run } from "cloudflare:test"; run(stub, (instance, state) => { state.storage.setAlarm = async () => {}; });
