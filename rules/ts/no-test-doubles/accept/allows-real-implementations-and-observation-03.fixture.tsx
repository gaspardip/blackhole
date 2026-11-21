import { env } from "cloudflare:workers"; function inspect(env) { return { ...env, onEvent() {} }; }
