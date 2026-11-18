import { env } from "cloudflare:workers"; env.AUTH = enabled ? native : { verify: () => null };
