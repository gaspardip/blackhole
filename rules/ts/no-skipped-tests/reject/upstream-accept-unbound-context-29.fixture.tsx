import { env } from "cloudflare:workers"; env.AUTH = enabled ? new Auth(context, env) : existing;
