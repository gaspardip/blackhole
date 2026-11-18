import { withEnv } from "cloudflare:workers"; const get = () => "token"; const method = "bind"; withEnv({ TOKEN: { get: get[method](null) } }, run);
