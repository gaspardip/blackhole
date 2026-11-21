import * as api from "vitest"; const tools = api.vi; tools.stubGlobal("fetch", request);
