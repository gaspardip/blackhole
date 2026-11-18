import { env } from "cloudflare:workers"; env.AUTH = { verify: () => null } ?? existing;
