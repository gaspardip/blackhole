import { withEnv } from "cloudflare:workers"; withEnv({ TOKEN: "token" }, async () => run());
