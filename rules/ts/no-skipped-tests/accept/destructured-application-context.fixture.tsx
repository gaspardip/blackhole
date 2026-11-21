import { env } from "cloudflare:workers"; function build({ context }) { env.AUTH = existing || new Auth(context, env); }
