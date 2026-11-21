import { env } from "cloudflare:workers"; const replacement = { get() { return "key"; } }; use({ ...env.TOKEN, ...replacement });
