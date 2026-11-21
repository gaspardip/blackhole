import { env } from "cloudflare:workers"; Reflect.defineProperty(env, "TOKEN", { value: { get: () => "token" } });
