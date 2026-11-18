import { env } from "cloudflare:workers"; function configure(env) { env.AUTH = { verify: () => null }; }
