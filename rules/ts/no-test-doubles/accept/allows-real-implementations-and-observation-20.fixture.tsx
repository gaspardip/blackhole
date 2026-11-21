import { env } from "cloudflare:workers"; function verify() {} function configure(verify) { env.AUTH = { verify: verify.bind(null) }; }
