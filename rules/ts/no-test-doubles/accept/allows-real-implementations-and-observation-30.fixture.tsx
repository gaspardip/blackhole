import { withEnv } from "cloudflare:workers"; withEnv(enabled ? { TOKEN: "token" } : {}, run);
