import { env } from "cloudflare:workers"; env.AUTH = { verify }; function verify() { return null; }
