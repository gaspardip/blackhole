import { env } from "cloudflare:workers"; Reflect.deleteProperty(env, "AUTH");
