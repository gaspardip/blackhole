import { env } from "cloudflare:workers"; const context = makeContext(); env.AUTH = new Auth(context, env);
