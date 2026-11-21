import { env } from "cloudflare:workers"; const bindings = env; Object.assign(bindings, { TOKEN: { get: async () => "token" } });
