import { env as bindings } from "cloudflare:workers"; const original = bindings; const key = { get: async () => "key" }; use({ ...original, TOKEN: key });
