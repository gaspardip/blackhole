import { withEnv } from "cloudflare:workers"; withEnv(enabled ? { AUTH: { verify: () => null } } : defaults, run);
