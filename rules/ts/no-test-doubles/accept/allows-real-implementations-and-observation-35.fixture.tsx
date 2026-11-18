import { env } from "cloudflare:workers"; env.AUTH = new Auth(context, env);
