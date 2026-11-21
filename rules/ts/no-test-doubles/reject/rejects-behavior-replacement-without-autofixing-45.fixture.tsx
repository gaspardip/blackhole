import { env } from "cloudflare:workers"; const replacement = enabled ? { verify: () => null } : native; Reflect.set(env, "AUTH", replacement);
