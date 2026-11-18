import { env } from "cloudflare:workers"; const replacement = { verify: () => null }; env.AUTH = { ...replacement };
