import { withEnv } from "cloudflare:workers"; withEnv({ AUTH: { verify: () => null } }, run);
