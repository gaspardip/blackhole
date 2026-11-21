import { env } from "cloudflare:workers"; function create() { return native; } env.AUTH = create();
