import { env } from "cloudflare:workers"; const replacement = { ...replacement, TOKEN: "token" }; Object.assign(env, replacement);
