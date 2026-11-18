import { env } from "cloudflare:workers"; env.AUTH = (prepare(), { verify: () => null });
