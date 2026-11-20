import { env } from "cloudflare:workers"; function build(context) { env.AUTH = enabled ? new Auth(context, env) : existing; }
