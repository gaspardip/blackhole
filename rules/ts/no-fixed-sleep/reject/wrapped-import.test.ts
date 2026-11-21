import {setTimeout as sleep} from "node:timers/promises"; it("x", async () => { await (sleep as typeof sleep)(100); });
