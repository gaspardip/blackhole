import { env } from "cloudflare:workers"; import { verify } from "./auth"; env.AUTH = { verify: verify.bind(null) };
