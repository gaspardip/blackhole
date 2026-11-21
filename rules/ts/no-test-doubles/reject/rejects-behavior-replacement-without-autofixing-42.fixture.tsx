import { env } from "cloudflare:workers"; env.AUTH = enabled && { verify: () => null };
