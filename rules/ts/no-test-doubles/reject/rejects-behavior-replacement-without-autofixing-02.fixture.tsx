import { env } from "cloudflare:workers"; use({ ...env, BOT_SIGNING_KEYS: { get: async () => "key" } });
