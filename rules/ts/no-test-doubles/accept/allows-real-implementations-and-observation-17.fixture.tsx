import { env } from "cloudflare:workers"; function verify() { return null; } function configure(verify) { env.AUTH = { verify }; }
