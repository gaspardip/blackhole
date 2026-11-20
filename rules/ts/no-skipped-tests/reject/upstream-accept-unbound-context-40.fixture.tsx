import { withEnv } from "cloudflare:workers"; withEnv({ AUTH: new Auth(context, env) }, run);
