import { env } from "cloudflare:workers"; const verify = () => null; env.AUTH = { verify: verify.bind(null) };
