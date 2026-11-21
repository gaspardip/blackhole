import { env } from "cloudflare:workers"; use({ ...env, TOKEN: env.OTHER_TOKEN });
