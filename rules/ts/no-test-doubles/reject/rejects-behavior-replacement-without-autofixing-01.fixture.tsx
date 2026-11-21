import { env, withEnv } from "cloudflare:workers"; withEnv({ ...env, TOKEN: { get: async () => "key" } }, run);
