import { env } from "cloudflare:workers"; function verify() {} const bound = verify.bind(null); env.AUTH.verify = bound;
