import { env } from "cloudflare:workers"; function verify() { return null; } verify = realVerify; env.AUTH = { verify };
