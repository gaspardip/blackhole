import { env } from "cloudflare:workers"; function verify(verify) { env.AUTH = { verify }; }
