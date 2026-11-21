import { withEnv } from "cloudflare:workers"; function get() { return "token"; } withEnv({ TOKEN: { get } }, run);
