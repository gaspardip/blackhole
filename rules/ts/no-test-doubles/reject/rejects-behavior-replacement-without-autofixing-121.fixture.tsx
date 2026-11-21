import * as api from "vitest"; const source = api; const { vitest: tools } = source; const alias = tools; alias.stubGlobal("fetch", request);
