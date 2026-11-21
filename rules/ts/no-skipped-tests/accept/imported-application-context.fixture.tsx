import { withEnv } from "cloudflare:workers"; import { context } from "./application"; withEnv({ AUTH: new Auth(context, env) }, run);
