import { env, withEnv } from "cloudflare:workers"; withEnv(({ ...env, fake: () => 1 }), run);
