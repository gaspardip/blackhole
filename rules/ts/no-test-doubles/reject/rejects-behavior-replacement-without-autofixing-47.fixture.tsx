import { env } from "cloudflare:workers"; Object.defineProperty(env.AUTH, "verify", { value: enabled ? () => null : native });
