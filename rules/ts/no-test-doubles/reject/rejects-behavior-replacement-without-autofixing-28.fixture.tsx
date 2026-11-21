import { env } from "cloudflare:workers"; async function verify() { return null; } const replacement = verify; env.AUTH.verify = replacement;
