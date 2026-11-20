import { env } from "cloudflare:workers"; env.AUTH = existing || new Auth(context, env);
