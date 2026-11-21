import { env } from "cloudflare:workers"; env.AUTH = { verify: async () => null };
