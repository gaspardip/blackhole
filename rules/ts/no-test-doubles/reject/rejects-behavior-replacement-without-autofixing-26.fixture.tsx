import { env } from "cloudflare:workers"; const verify = function () {}; Object.assign(env.AUTH, { verify: verify.bind(null) });
