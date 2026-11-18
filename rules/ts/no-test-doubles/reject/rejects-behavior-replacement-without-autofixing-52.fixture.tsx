import { env } from "cloudflare:workers"; Reflect.set(env, "AUTH", { verify() { return null; } });
