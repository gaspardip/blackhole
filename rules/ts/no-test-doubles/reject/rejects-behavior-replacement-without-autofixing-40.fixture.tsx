import { env } from "cloudflare:workers"; env.AUTH = existing || { verify: () => null };
